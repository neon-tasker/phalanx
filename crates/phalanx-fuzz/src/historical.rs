//! Historical workload fixture matching Ethereum Mainnet Block 14682499.

use crate::differential::allocate_static_statuses;
use alloy_primitives::{address, Address, TxKind, U256};
use phalanx_core::{MutationOp, StorageTarget};
use phalanx_pipeline::dispatcher::CommutativeTxEnvelope;
use revm::primitives::TxEnv;

/// Block 14682499 fixture.
pub struct OthersideBlock14682499Fixture {
    /// Otherdeed contract.
    pub contract_address: Address,
    /// Total supply slot.
    pub total_supply_slot: U256,
    /// Pre-state supply.
    pub initial_total_supply: U256,
    /// Workload count.
    pub count: usize,
}

impl OthersideBlock14682499Fixture {
    /// Loads fixture.
    pub fn load_workload(count: usize) -> Self {
        Self {
            contract_address: address!("34d85c9CDeB23FA97cb08333b511ac86E1C4E258"),
            total_supply_slot: U256::from(0x07),
            initial_total_supply: U256::from(20_000u64),
            count,
        }
    }

    /// Converts fixture to envelopes.
    pub fn into_envelopes(&self) -> Vec<CommutativeTxEnvelope> {
        let statuses = allocate_static_statuses(self.count);
        let target = StorageTarget::new(self.contract_address, self.total_supply_slot);

        (0..self.count)
            .map(|i| {
                let mut addr_bytes = [0u8; 20];
                addr_bytes[16..20].copy_from_slice(&(i as u32 + 1).to_be_bytes());
                addr_bytes[0] = 0xAA;
                addr_bytes[1] = (i % 250) as u8;

                let minter = Address::from(addr_bytes);
                let mint_count = if i % 3 == 0 { U256::from(2u64) } else { U256::from(1u64) };

                let mut tx = TxEnv::default();
                tx.caller = minter;
                tx.transact_to = TxKind::Call(self.contract_address);
                tx.gas_limit = 150_000;

                CommutativeTxEnvelope {
                    tx,
                    tx_index: i,
                    target,
                    op: MutationOp::Add,
                    delta: mint_count,
                    recipient: minter,
                    base_gas: 21_000,
                    status: &statuses[i],
                }
            })
            .collect()
    }
}