use alloy_primitives::{address, Address, TxKind, U256};
use phalanx_core::{MutationOp, StorageTarget};
use phalanx_enclave::filter::LockFreeDisjointFilter;
use phalanx_fuzz::differential::{allocate_static_statuses, DifferentialTestRunner};
use phalanx_pipeline::dispatcher::CommutativeTxEnvelope;
use proptest::prelude::*;
use revm::primitives::TxEnv;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn fuzz_randomized_mint_batches(
        deltas in prop::collection::vec(1u64..1_000_000_000u64, 1..32),
        initial_val_raw in 0u64..1_000_000_000u64,
    ) {
        let count = deltas.len();
        let statuses = allocate_static_statuses(count);
        let target = StorageTarget::new(address!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"), U256::from(0x42));
        let initial_val = U256::from(initial_val_raw);

        let filter = LockFreeDisjointFilter::new();
        let mut envelopes = Vec::with_capacity(count);
        let mut addr_id: u32 = 100;

        for (i, delta_val) in deltas.into_iter().enumerate() {
            let recipient = loop {
                let mut addr_bytes = [0u8; 20];
                addr_bytes[16..20].copy_from_slice(&addr_id.to_be_bytes());
                addr_bytes[0] = 0xFE;
                let candidate = Address::from(addr_bytes);
                addr_id += 1;
                if filter.try_acquire(&candidate) {
                    break candidate;
                }
            };

            let mut tx = TxEnv::default();
            tx.caller = recipient;
            tx.transact_to = TxKind::Call(target.address);
            tx.gas_limit = 100_000;

            envelopes.push(CommutativeTxEnvelope {
                tx,
                tx_index: i,
                target,
                op: MutationOp::Add,
                delta: U256::from(delta_val),
                recipient,
                base_gas: 21_000,
                status: &statuses[i],
            });
        }

        let diff_res = DifferentialTestRunner::assert_identical_execution(target, initial_val, envelopes, 4);
        prop_assert!(diff_res.is_ok());
    }
}