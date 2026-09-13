//! Foundational Primitives, Types, and Abelian Invariant Math for the Phalanx Engine.

#![deny(missing_docs)]
#![deny(unsafe_code)]

use alloy_primitives::{Address, U256};
use thiserror::Error;

/// Target storage slot inside the world state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageTarget {
    /// Contract account address.
    pub address: Address,
    /// Storage slot key.
    pub slot: U256,
}

impl StorageTarget {
    /// Constructs a new `StorageTarget`.
    #[inline(always)]
    pub const fn new(address: Address, slot: U256) -> Self {
        Self { address, slot }
    }
}

/// Permitted monotonic storage mutations admitted to the commutative fast-path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutationOp {
    /// Additive delta conforming to monotonic increments.
    Add,
    /// Subtractive delta conforming to bounded monotonic decrements.
    Sub,
}

/// Represents an isolated, state-independent storage mutation emitted by a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommutativeDelta {
    /// Target storage address and slot key.
    pub target: StorageTarget,
    /// Arithmetic operator applied.
    pub op: MutationOp,
    /// Unsigned 256-bit magnitude of the mutation.
    pub value: U256,
    /// Canonical transaction index within the block execution batch.
    pub tx_index: usize,
}

impl CommutativeDelta {
    /// Instantiates a new `CommutativeDelta`.
    #[inline(always)]
    pub const fn new(target: StorageTarget, op: MutationOp, value: U256, tx_index: usize) -> Self {
        Self { target, op, value, tx_index }
    }
}

/// Strict mathematical invariant violation errors for commutative storage mutations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum InvariantError {
    /// Addition exceeded the finite field ceiling 2^256 - 1.
    #[error("Arithmetic overflow detected on target {target:?}: current {current}, delta {delta}")]
    Overflow {
        /// Storage location where overflow occurred.
        target: StorageTarget,
        /// Current base value before addition.
        current: U256,
        /// Attempted addition magnitude.
        delta: U256,
    },
    /// Subtraction breached below zero in unsigned arithmetic.
    #[error("Arithmetic underflow detected on target {target:?}: current {current}, delta {delta}")]
    Underflow {
        /// Storage location where underflow occurred.
        target: StorageTarget,
        /// Current base value before subtraction.
        current: U256,
        /// Attempted subtraction magnitude.
        delta: U256,
    },
}

/// Applies a commutative addition over (Z_{2^256}, +) with checked overflow bounds.
#[inline(always)]
pub fn checked_abelian_add(current: U256, delta: U256, target: StorageTarget) -> Result<U256, InvariantError> {
    let (res, overflow) = current.overflowing_add(delta);
    if overflow {
        Err(InvariantError::Overflow { target, current, delta })
    } else {
        Ok(res)
    }
}

/// Applies a checked decrement over (Z_{2^256}, -) verifying non-negative boundaries.
#[inline(always)]
pub fn checked_abelian_sub(current: U256, delta: U256, target: StorageTarget) -> Result<U256, InvariantError> {
    let (res, underflow) = current.overflowing_sub(delta);
    if underflow {
        Err(InvariantError::Underflow { target, current, delta })
    } else {
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abelian_group_commutativity_invariant() {
        let target = StorageTarget::new(Address::ZERO, U256::from(1));
        let v_initial = U256::from(1_000_000u64);
        let delta_a = U256::from(450_500u64);
        let delta_b = U256::from(890_120u64);

        let step1_p1 = checked_abelian_add(v_initial, delta_a, target).expect("valid add");
        let final_p1 = checked_abelian_add(step1_p1, delta_b, target).expect("valid add");

        let step1_p2 = checked_abelian_add(v_initial, delta_b, target).expect("valid add");
        let final_p2 = checked_abelian_add(step1_p2, delta_a, target).expect("valid add");

        assert_eq!(final_p1, final_p2);
        assert_eq!(final_p1, U256::from(2_340_620u64));
    }

    #[test]
    fn test_abelian_overflow_detection() {
        let target = StorageTarget::new(Address::ZERO, U256::ZERO);
        let err = checked_abelian_add(U256::MAX, U256::from(1u64), target).unwrap_err();
        match err {
            InvariantError::Overflow { current, delta, .. } => {
                assert_eq!(current, U256::MAX);
                assert_eq!(delta, U256::from(1u64));
            }
            _ => panic!("Expected InvariantError::Overflow"),
        }
    }
}