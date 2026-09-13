//! Differential fuzz testing, state root assertions, and historical Ethereum Mainnet replay fixtures.

#![deny(missing_docs)]

pub mod differential;
pub mod historical;

pub use differential::{allocate_static_statuses, build_hotspot_mint_bytecode, DifferentialTestRunner, SequentialRevmRunner};
pub use historical::OthersideBlock14682499Fixture;