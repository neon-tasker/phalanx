//! Microarchitectural Memory Isolation, Silicon Barrier Reduction, and Receipt Scanning.

#![deny(missing_docs)]
#![cfg_attr(test, allow(missing_docs))]

pub mod buffer;
pub mod receipt;
pub mod reduction;

pub use buffer::{AlignedSlotAccumulator, HotspotAccumulatorArena};
pub use receipt::{CanonicalReceipt, DeterministicReceiptPipeline, RawExecutionRecord, ThreadExecutionArena};
pub use reduction::{reduce_16, reduce_scalar_fallback};