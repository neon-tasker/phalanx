use alloy_primitives::{address, Address, B256, U256};
use phalanx_accumulator::buffer::AlignedSlotAccumulator;
use phalanx_accumulator::reduction::reduce_16;
use phalanx_enclave::filter::LockFreeDisjointFilter;
use phalanx_enclave::scanner::{BytecodeScanner, EnclaveRejection};

#[test]
fn test_create2_mid_batch_frontrunning_rejection() {
    let filter = LockFreeDisjointFilter::new();
    let deployer = address!("000000000000000000000000000000000000c0de");
    let salt = B256::from(U256::from(0xbeef));
    let init_code = vec![0x60, 0x00, 0x60, 0x00, 0xf3];
    let init_code_hash = alloy_primitives::keccak256(&init_code);

    let mut create2_preimage = Vec::with_capacity(1 + 20 + 32 + 32);
    create2_preimage.push(0xff);
    create2_preimage.extend_from_slice(deployer.as_slice());
    create2_preimage.extend_from_slice(salt.as_slice());
    create2_preimage.extend_from_slice(init_code_hash.as_slice());

    let deployed_address = Address::from_slice(&alloy_primitives::keccak256(&create2_preimage)[12..32]);

    assert!(filter.try_acquire(&deployed_address));
    assert!(!filter.try_acquire(&deployed_address));
}

#[test]
fn test_obfuscated_sload_swap_jumpi_rejection() {
    let malicious_bytecode = vec![0x60, 0x00, 0x54, 0x60, 0x01, 0x90, 0x60, 0x20, 0x57, 0x00];
    let result = BytecodeScanner::verify_monotonic_blind_accumulator(&malicious_bytecode, U256::ZERO);
    assert_eq!(result, Err(EnclaveRejection::TaintedConditionalBranch { pc: 8 }));
}

#[test]
fn test_atomic_unwind_on_gas_exhaustion_boundary() {
    let accumulator = AlignedSlotAccumulator::new();
    let initial_delta = U256::from(50_000u64);
    let initial_gas = 100_000u64;

    unsafe {
        accumulator.record_delta(initial_delta, initial_gas);
        accumulator.unwind_delta(initial_delta, initial_gas);
        assert_eq!(accumulator.read_delta(), U256::ZERO);
        assert_eq!(accumulator.read_gas(), 0);
    }

    let arena = [(); 16].map(|_| AlignedSlotAccumulator::new());
    unsafe {
        arena[0].record_delta(accumulator.read_delta(), accumulator.read_gas());
    }

    let (reduced_sum, overflow) = reduce_16(&arena);
    assert_eq!(reduced_sum, U256::ZERO);
    assert!(!overflow);
}