//! Deterministic Soak & Endurance Testing Harness for Phalanx Engine.

use alloy_primitives::{address, Address, TxKind, U256};
use phalanx_core::{MutationOp, StorageTarget};
use phalanx_fuzz::differential::allocate_static_statuses;
use phalanx_pipeline::dispatcher::{tx_status, CommutativeTxEnvelope, DualPathDispatcher};
use revm::primitives::TxEnv;
use std::sync::atomic::Ordering;
use std::time::Instant;

struct FastPrng(u64);
impl FastPrng {
    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

#[test]
fn test_soak_endurance() {
    let target_tx_count: u64 = std::env::var("SOAK_TX_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(25_000);

    let batch_size: usize = 5_000;
    let num_batches = ((target_tx_count as usize) + batch_size - 1) / batch_size;

    let oversubscribed_threads = 32;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(oversubscribed_threads)
        .build()
        .expect("Failed to initialize Rayon pool");

    let dispatcher = DualPathDispatcher::new(16);
    let target = StorageTarget::new(
        address!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
        U256::from(0x42),
    );

    let statuses = allocate_static_statuses(batch_size);
    let mut prng = FastPrng(0x1337BEEF_CAFEF00D);
    let mut current_slot_val = U256::from(1_000_000_000u64);

    println!("======================================================================");
    println!("PHALANX ENDURANCE SOAK TEST HARNESS");
    println!("Target Volume:       {} txs across {} batches", target_tx_count, num_batches);
    println!("Batch Granularity:   {} tx/batch", batch_size);
    println!("Rayon Pool:          {} worker threads", oversubscribed_threads);
    println!("======================================================================");

    let start_time = Instant::now();

    for batch_idx in 0..num_batches {
        for s in statuses.iter() {
            s.store(tx_status::STATUS_COMMUTATIVE, Ordering::Relaxed);
        }

        let mut batch = Vec::with_capacity(batch_size);
        let mut expected_batch_delta = U256::ZERO;

        for i in 0..batch_size {
            let mut addr_bytes = [0u8; 20];
            let rand_val = prng.next_u64();
            addr_bytes[0..8].copy_from_slice(&rand_val.to_be_bytes());
            addr_bytes[16..20].copy_from_slice(&(i as u32).to_be_bytes());
            let recipient = Address::from(addr_bytes);

            let delta_val = (prng.next_u64() % 1_000_000) + 1;
            let delta = U256::from(delta_val);
            expected_batch_delta = expected_batch_delta.wrapping_add(delta);

            let mut tx = TxEnv::default();
            tx.caller = recipient;
            tx.transact_to = TxKind::Call(target.address);
            tx.gas_limit = 100_000;

            batch.push(CommutativeTxEnvelope {
                tx,
                tx_index: i,
                target,
                op: MutationOp::Add,
                delta,
                recipient,
                base_gas: 21_000,
                status: &statuses[i],
            });
        }

        let result = pool
            .install(|| dispatcher.dispatch_batch(batch, current_slot_val))
            .expect("Dispatcher batch execution must succeed");

        // Dual-Path Reconciliation:
        // Transactions admitted to the fast-path are accumulated in final_slot_value.
        // Transactions that experienced partition collisions are safely routed to
        // ejected_transactions for sequential fallback execution.
        let mut fallback_delta = U256::ZERO;
        for ejected_tx in &result.ejected_transactions {
            fallback_delta = fallback_delta.wrapping_add(ejected_tx.delta);
        }

        let total_block_final = result.final_slot_value.wrapping_add(fallback_delta);
        let expected_final = current_slot_val.wrapping_add(expected_batch_delta);

        assert_eq!(
            total_block_final, expected_final,
            "State divergence in batch {}: expected {}, got {}",
            batch_idx, expected_final, total_block_final
        );
        assert_eq!(
            result.receipts.len() + result.ejected_transactions.len(),
            batch_size,
            "Total executed plus ejected transactions must equal batch size"
        );

        current_slot_val = total_block_final;
    }

    let elapsed = start_time.elapsed();
    let throughput = (num_batches * batch_size) as f64 / elapsed.as_secs_f64();
    println!("Soak execution completed: {} txs in {:.3?} ({:.0} tx/s)", num_batches * batch_size, elapsed, throughput);
}