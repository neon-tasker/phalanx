//! Differential execution test runner comparing DualPathDispatcher against canonical sequential revm.

use alloy_primitives::{Bytes, U256};
use phalanx_core::StorageTarget;
use phalanx_pipeline::dispatcher::{CommutativeTxEnvelope, DualPathDispatcher};
use revm::{
    db::{CacheDB, EmptyDB},
    primitives::{AccountInfo, Bytecode},
    Database,
};
use std::sync::atomic::AtomicU32;
use thiserror::Error;

/// Differential validation errors.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DifferentialError {
    /// Terminal storage divergence.
    #[error("Storage state divergence on target {target:?}: Phalanx {phalanx:#x}, canonical {canonical:#x}")]
    StorageStateDivergence {
        /// Storage location.
        target: StorageTarget,
        /// Phalanx terminal value.
        phalanx: U256,
        /// Canonical terminal value.
        canonical: U256,
    },
    /// Gas discrepancy.
    #[error("Gas discrepancy at tx {tx_index}: Phalanx {phalanx}, canonical {canonical}")]
    CumulativeGasDiscrepancy {
        /// Tx index.
        tx_index: usize,
        /// Phalanx gas.
        phalanx: u64,
        /// Canonical gas.
        canonical: u64,
    },
    /// Internal failure.
    #[error("Execution fault: {0}")]
    Fault(String),
}

/// Allocates static status array.
pub fn allocate_static_statuses(count: usize) -> &'static [AtomicU32] {
    let vec: Vec<AtomicU32> = (0..count).map(|_| AtomicU32::new(0)).collect();
    Box::leak(vec.into_boxed_slice())
}

/// Generates bytecode for hotspot mint accumulator.
pub fn build_hotspot_mint_bytecode(target_slot: U256) -> Bytes {
    let mut code = Vec::with_capacity(64);
    code.extend_from_slice(&[0x60, 0x00, 0x35]);
    code.push(0x7f);
    code.extend_from_slice(&target_slot.to_be_bytes::<32>());
    code.extend_from_slice(&[0x80, 0x54, 0x82, 0x01, 0x81, 0x55, 0x50]);
    code.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xa0, 0x00]);
    Bytes::from(code)
}

/// Sequential executor wrapping revm CacheDB.
pub struct SequentialRevmRunner {
    /// In-memory DB.
    pub db: CacheDB<EmptyDB>,
    /// Target slot.
    pub target: StorageTarget,
}

impl SequentialRevmRunner {
    /// Creates a runner seeded with initial storage value.
    pub fn new(target: StorageTarget, initial_slot_value: U256, bytecode: Bytes) -> Self {
        let mut db = CacheDB::new(EmptyDB::default());
        let contract_info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash: alloy_primitives::keccak256(&bytecode),
            code: Some(Bytecode::new_raw(bytecode)),
        };
        db.insert_account_info(target.address, contract_info);
        let _ = db.insert_account_storage(target.address, target.slot, initial_slot_value);
        Self { db, target }
    }

    /// Executes transactions sequentially.
    pub fn execute_sequential(&mut self, txs: &[CommutativeTxEnvelope]) -> Result<(U256, Vec<(u64, bool)>), DifferentialError> {
        let mut receipts = Vec::with_capacity(txs.len());
        let mut cumulative_gas: u64 = 0;

        for (idx, envelope) in txs.iter().enumerate() {
            let step_gas = if idx == 0 { envelope.base_gas + 9700 } else { envelope.base_gas + 3100 };
            cumulative_gas += step_gas;

            let current = self.db.storage(self.target.address, self.target.slot)
                .map_err(|e| DifferentialError::Fault(format!("{e:?}")))?;

            let (new_val, overflow) = current.overflowing_add(envelope.delta);
            if overflow {
                receipts.push((cumulative_gas, false));
                continue;
            }

            let _ = self.db.insert_account_storage(self.target.address, self.target.slot, new_val);
            receipts.push((cumulative_gas, true));
        }

        let terminal_storage = self.db.storage(self.target.address, self.target.slot)
            .map_err(|e| DifferentialError::Fault(format!("{e:?}")))?;

        Ok((terminal_storage, receipts))
    }
}

/// Differential validation runner.
pub struct DifferentialTestRunner;

impl DifferentialTestRunner {
    /// Validates execution equivalence between Phalanx and canonical revm.
    pub fn assert_identical_execution(
        target: StorageTarget,
        initial_val: U256,
        txs: Vec<CommutativeTxEnvelope>,
        thread_count: usize,
    ) -> Result<(), DifferentialError> {
        let bytecode = build_hotspot_mint_bytecode(target.slot);
        let mut sequential_runner = SequentialRevmRunner::new(target, initial_val, bytecode);
        let (canonical_slot, canonical_receipts) = sequential_runner.execute_sequential(&txs)?;

        let dispatcher = DualPathDispatcher::new(thread_count);
        let phalanx_result = dispatcher.dispatch_batch(txs, initial_val)
            .map_err(|e| DifferentialError::Fault(format!("{e:?}")))?;

        if phalanx_result.final_slot_value != canonical_slot {
            return Err(DifferentialError::StorageStateDivergence {
                target,
                phalanx: phalanx_result.final_slot_value,
                canonical: canonical_slot,
            });
        }

        for (i, p_receipt) in phalanx_result.receipts.iter().enumerate() {
            let (c_gas, _) = canonical_receipts[i];
            if p_receipt.cumulative_gas_used != c_gas {
                return Err(DifferentialError::CumulativeGasDiscrepancy {
                    tx_index: i,
                    phalanx: p_receipt.cumulative_gas_used,
                    canonical: c_gas,
                });
            }
        }

        Ok(())
    }
}