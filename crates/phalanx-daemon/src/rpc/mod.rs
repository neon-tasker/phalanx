//! JSON-RPC protocol handling, request routing, and error schemas.
pub mod router;
pub mod types;

pub use router::{build_router, AppState};
pub use types::{CallObject, JsonRpcError, JsonRpcPayload, JsonRpcRequest, JsonRpcResponse};