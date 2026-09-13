//! Commutative Fast-Path Pipeline, Consensus Scheduler, and EIP-2929 Reconciler.

#![deny(missing_docs)]

pub mod dispatcher;

pub use dispatcher::{CommutativeTxEnvelope, DispatchResult, DualPathDispatcher};