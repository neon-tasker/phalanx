//! Static Bytecode Abstract Interpretation and Lock-Free Disjoint Partition Filters.

#![deny(missing_docs)]

pub mod filter;
pub mod scanner;

pub use filter::{is_eoa_recipient, LockFreeDisjointFilter, KECCAK_EMPTY};
pub use scanner::{BytecodeScanner, EnclaveRejection};