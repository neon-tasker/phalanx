//! Phalanx Daemon: In-memory EVM simulation proxy with speculative JIT state hydration.
pub mod engine;
pub mod rpc;

pub use engine::{PipelinedJitDb, SimulationExecutor};
pub use rpc::{build_router, AppState, JsonRpcRequest, JsonRpcResponse};