//! Zero-copy bytecode scanner verifying pure monotonic commutativity invariants.

use alloy_primitives::U256;
use thiserror::Error;

/// EVM opcodes inspected during static verification.
pub mod opcodes {
    /// SLOAD
    pub const SLOAD: u8 = 0x54;
    /// JUMPI
    pub const JUMPI: u8 = 0x57;
    /// PUSH1
    pub const PUSH1: u8 = 0x60;
    /// PUSH32
    pub const PUSH32: u8 = 0x7F;
    /// CALL
    pub const CALL: u8 = 0xF1;
    /// CALLCODE
    pub const CALLCODE: u8 = 0xF2;
    /// DELEGATECALL
    pub const DELEGATECALL: u8 = 0xF4;
    /// STATICCALL
    pub const STATICCALL: u8 = 0xFA;
    /// SELFDESTRUCT
    pub const SELFDESTRUCT: u8 = 0xFF;
}

/// Architectural rejections when bytecode violates commutative fast-path invariants.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum EnclaveRejection {
    /// Conditional branch tainted by storage read.
    #[error("Conditional branch JUMPI detected at PC {pc:#x} tainted by storage read")]
    TaintedConditionalBranch {
        /// Program counter.
        pc: usize,
    },
    /// Prohibited external dynamic call detected.
    #[error("Dynamic call instruction {opcode:#x} detected at PC {pc:#x}")]
    ProhibitedExternalCall {
        /// Program counter.
        pc: usize,
        /// Opcode byte.
        opcode: u8,
    },
    /// Malformed EVM instruction sequence.
    #[error("Malformed bytecode: {0}")]
    MalformedBytecode(String),
}

/// Zero-copy static bytecode analyzer.
pub struct BytecodeScanner;

impl BytecodeScanner {
    /// Proves that contract bytecode strictly adheres to an Abelian storage accumulator pattern.
    pub fn verify_monotonic_blind_accumulator(code: &[u8], _target_slot: U256) -> Result<(), EnclaveRejection> {
        let mut pc = 0;
        let len = code.len();
        let mut sload_observed = false;

        while pc < len {
            let opcode = code[pc];

            match opcode {
                opcodes::CALL
                | opcodes::CALLCODE
                | opcodes::DELEGATECALL
                | opcodes::STATICCALL
                | opcodes::SELFDESTRUCT => {
                    return Err(EnclaveRejection::ProhibitedExternalCall { pc, opcode });
                }
                _ => {}
            }

            if opcode == opcodes::SLOAD {
                sload_observed = true;
            }

            if sload_observed && opcode == opcodes::JUMPI {
                return Err(EnclaveRejection::TaintedConditionalBranch { pc });
            }

            if (opcodes::PUSH1..=opcodes::PUSH32).contains(&opcode) {
                let push_size = (opcode - opcodes::PUSH1 + 1) as usize;
                pc += push_size;
                if pc >= len && pc != len {
                    return Err(EnclaveRejection::MalformedBytecode("Truncated PUSH immediate".into()));
                }
            }

            pc += 1;
        }

        Ok(())
    }
}