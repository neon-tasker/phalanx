//! In-memory revm database and speculative fault hydrator.
pub mod executor;
pub mod trap_db;

pub use executor::SimulationExecutor;
pub use trap_db::{DatabaseError, PipelinedJitDb, SessionDb};