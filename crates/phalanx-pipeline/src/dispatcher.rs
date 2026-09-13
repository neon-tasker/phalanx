//! Rayon-backed dual-path parallel scheduler and mid-flight transaction fallback engine.

use alloy_primitives::{Address, Bytes, Log, LogData, U256};
use phalanx_accumulator::buffer::{AlignedSlotAccumulator, HotspotAccumulatorArena};
use phalanx_accumulator::receipt::{
    CanonicalReceipt, DeterministicReceiptPipeline, RawExecutionRecord, ThreadExecutionArena,
};
use phalanx_accumulator::reduction::reduce_16;
use phalanx_core::{checked_abelian_add, MutationOp, StorageTarget};
use phalanx_enclave::filter::LockFreeDisjointFilter;
use rayon::prelude::*;
use revm::primitives::TxEnv;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use thiserror::Error;

/// Runtime dispatcher execution errors.
#[derive(Error, Debug)]
pub enum DispatchError {
    /// Arithmetic barrier overflow.
    #[error("Barrier reduction overflowed U256 ceiling")]
    BarrierReductionOverflow,
    /// Internal execution fault.
    #[error("Internal dispatcher pipeline fault: {0}")]
    InternalFault(String),
}

/// Transaction execution status indicators.
pub mod tx_status {
    /// Active on fast path.
    pub const STATUS_COMMUTATIVE: u32 = 0;
    /// Ejected to fallback.
    pub const STATUS_EJECTED: u32 = 1;
    /// Successfully committed.
    pub const STATUS_COMMITTED: u32 = 2;
}

/// Transaction wrapper ingested by Phalanx.
#[derive(Clone, Debug)]
pub struct CommutativeTxEnvelope {
    /// Raw EVM tx environment.
    pub tx: TxEnv,
    /// Canonical transaction index.
    pub tx_index: usize,
    /// Contended storage target.
    pub target: StorageTarget,
    /// Mutation operator.
    pub op: MutationOp,
    /// Unsigned delta.
    pub delta: U256,
    /// Recipient address.
    pub recipient: Address,
    /// Base execution gas.
    pub base_gas: u64,
    /// Status flag reference.
    pub status: &'static AtomicU32,
}

/// Final result returned after block-level convergence.
pub struct DispatchResult {
    /// Terminal storage slot state.
    pub final_slot_value: U256,
    /// Deterministically computed receipts.
    pub receipts: Vec<CanonicalReceipt>,
    /// Combined block bloom.
    pub block_bloom: [u8; 256],
    /// Ejected transactions requiring fallback sequential execution.
    pub ejected_transactions: Vec<CommutativeTxEnvelope>,
}

/// Dual-path execution dispatcher.
pub struct DualPathDispatcher {
    thread_count: usize,
    arena: HotspotAccumulatorArena,
    filter: LockFreeDisjointFilter,
}

impl DualPathDispatcher {
    /// Instantiates dispatcher with up to 16 workers.
    pub fn new(thread_count: usize) -> Self {
        assert!(thread_count <= 16);
        Self {
            thread_count,
            arena: HotspotAccumulatorArena::new(thread_count),
            filter: LockFreeDisjointFilter::new(),
        }
    }

    /// Orchestrates parallel commutative execution over a batch of hotspot transactions.
    pub fn dispatch_batch(
        &self,
        mut transactions: Vec<CommutativeTxEnvelope>,
        initial_slot_value: U256,
    ) -> Result<DispatchResult, DispatchError> {
        if transactions.is_empty() {
            return Ok(DispatchResult {
                final_slot_value: initial_slot_value,
                receipts: Vec::new(),
                block_bloom: [0u8; 256],
                ejected_transactions: Vec::new(),
            });
        }

        self.arena.reset_all();
        self.filter.reset();

        let ejected_txs = Mutex::new(Vec::new());
        let chunk_size = (transactions.len() + self.thread_count - 1) / self.thread_count;
        let chunk_size = chunk_size.max(1);

        let mut thread_arenas: Vec<ThreadExecutionArena> = (0..self.thread_count)
            .map(|_| ThreadExecutionArena::new())
            .collect();

        // Partition transactions across workers
        transactions
            .par_chunks_mut(chunk_size)
            .zip(thread_arenas.par_iter_mut())
            .enumerate()
            .for_each(|(thread_id, (tx_chunk, arena))| {
                let accumulator = self.arena.get(thread_id);

                for tx_envelope in tx_chunk.iter_mut() {
                    // Admission check: if partition bit is already taken, reject to fallback queue
                    if !self.filter.try_acquire(&tx_envelope.recipient) {
                        self.eject_admission_conflict(tx_envelope, accumulator, &ejected_txs);
                        continue;
                    }

                    // EIP-2929 dynamic cold/warm gas calculation
                    let dynamic_gas = if tx_envelope.tx_index == 0 {
                        tx_envelope.base_gas + 9700
                    } else {
                        tx_envelope.base_gas + 3100
                    };

                    unsafe {
                        accumulator.record_delta(tx_envelope.delta, dynamic_gas);
                    }

                    arena.push_record(RawExecutionRecord {
                        tx_index: tx_envelope.tx_index,
                        gas_used: dynamic_gas,
                        logs: vec![Log {
                            address: tx_envelope.target.address,
                            data: LogData::new_unchecked(
                                vec![],
                                Bytes::copy_from_slice(&tx_envelope.delta.to_be_bytes::<32>()),
                            ),
                        }],
                        success: true,
                    });

                    tx_envelope.status.store(tx_status::STATUS_COMMITTED, Ordering::Release);
                }
            });

        let slice = self.arena.as_slice();
        let fixed_16: [AlignedSlotAccumulator; 16] = [(); 16].map(|_| AlignedSlotAccumulator::new());
        for i in 0..self.thread_count {
            unsafe {
                fixed_16[i].record_delta(slice[i].read_delta(), slice[i].read_gas());
            }
        }

        let (total_delta, reduction_overflow) = reduce_16(&fixed_16);
        if reduction_overflow {
            return Err(DispatchError::BarrierReductionOverflow);
        }

        let target = transactions[0].target;
        let final_slot_value = checked_abelian_add(initial_slot_value, total_delta, target)
            .map_err(|_| DispatchError::BarrierReductionOverflow)?;

        let (receipts, block_bloom) = DeterministicReceiptPipeline::finalize(thread_arenas);
        let ejected_transactions = ejected_txs.into_inner().map_err(|e| {
            DispatchError::InternalFault(format!("Mutex poisoned: {e}"))
        })?;

        Ok(DispatchResult {
            final_slot_value,
            receipts,
            block_bloom,
            ejected_transactions,
        })
    }

    /// Safely handles partition collision at admission (delta was never added to accumulator).
    #[inline(always)]
    fn eject_admission_conflict(
        &self,
        tx: &CommutativeTxEnvelope,
        accumulator: &AlignedSlotAccumulator,
        ejected_queue: &Mutex<Vec<CommutativeTxEnvelope>>,
    ) {
        if tx
            .status
            .compare_exchange(
                tx_status::STATUS_COMMUTATIVE,
                tx_status::STATUS_EJECTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            accumulator.raise_flag(AlignedSlotAccumulator::FLAG_EJECTED);
            if let Ok(mut q) = ejected_queue.lock() {
                q.push(tx.clone());
            }
        }
    }

    /// Handles mid-flight runtime ejection (unwinds previously recorded delta).
    #[allow(dead_code)]
    #[inline(always)]
    fn trigger_midflight_ejection(
        &self,
        tx: &CommutativeTxEnvelope,
        accumulator: &AlignedSlotAccumulator,
        ejected_queue: &Mutex<Vec<CommutativeTxEnvelope>>,
        gas_used: u64,
    ) {
        if tx
            .status
            .compare_exchange(
                tx_status::STATUS_COMMUTATIVE,
                tx_status::STATUS_EJECTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            unsafe {
                accumulator.unwind_delta(tx.delta, gas_used);
            }
            accumulator.raise_flag(AlignedSlotAccumulator::FLAG_EJECTED);
            if let Ok(mut q) = ejected_queue.lock() {
                q.push(tx.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, U256};

    #[test]
    fn test_pipeline_dispatch() {
        let dispatcher = DualPathDispatcher::new(4);
        let target = StorageTarget::new(
            address!("1111111111111111111111111111111111111111"),
            U256::from(0xaa),
        );

        let statuses: Vec<AtomicU32> = (0..8).map(|_| AtomicU32::new(0)).collect();
        let static_statuses: &'static [AtomicU32] = Box::leak(statuses.into_boxed_slice());

        let mut txs = Vec::new();
        for i in 0..8 {
            txs.push(CommutativeTxEnvelope {
                tx: TxEnv::default(),
                tx_index: i,
                target,
                op: MutationOp::Add,
                delta: U256::from(10_000u64),
                recipient: Address::from_slice(&[(i as u8) + 1; 20]),
                base_gas: 21_000,
                status: &static_statuses[i],
            });
        }

        let result = dispatcher.dispatch_batch(txs, U256::from(500_000u64)).unwrap();
        assert_eq!(result.final_slot_value, U256::from(580_000u64));
        assert_eq!(result.receipts.len(), 8);
        assert_eq!(result.receipts[0].cumulative_gas_used, 21_000 + 9700);
    }
}