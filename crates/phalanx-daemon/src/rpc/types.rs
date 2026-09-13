//! Strongly typed JSON-RPC 2.0 message schemas.
use alloy_primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Discriminated payload supporting both single and batch JSON-RPC calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcPayload {
    Single(JsonRpcRequest),
    Batch(Vec<JsonRpcRequest>),
}

/// Standard JSON-RPC 2.0 request frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Standard JSON-RPC 2.0 response frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Spec-compliant JSON-RPC error structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".to_string(),
            data: None,
        }
    }

    pub fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }

    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }

    pub fn execution_reverted(data_hex: String) -> Self {
        Self {
            code: 3,
            message: format!("execution reverted: {}", data_hex),
            data: Some(Value::String(data_hex)),
        }
    }
}

/// Normalized EVM call object schema supporting both `data` and `input` fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CallObject {
    pub from: Option<Address>,
    pub to: Option<Address>,
    pub gas: Option<U256>,
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub value: Option<U256>,
    pub data: Option<Bytes>,
    pub input: Option<Bytes>,
    pub nonce: Option<u64>,
    pub chain_id: Option<u64>,
}

impl CallObject {
    pub fn calldata(&self) -> Bytes {
        if let Some(ref d) = self.data {
            d.clone()
        } else if let Some(ref i) = self.input {
            i.clone()
        } else {
            Bytes::default()
        }
    }
}