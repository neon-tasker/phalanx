//! Axum-based JSON-RPC proxy router dispatching local simulations and upstream pass-throughs.
use crate::engine::SimulationExecutor;
use crate::rpc::types::{JsonRpcError, JsonRpcPayload, JsonRpcRequest, JsonRpcResponse};
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    pub upstream_url: String,
    pub http_client: reqwest::Client,
    pub executor: Arc<SimulationExecutor>,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT]);

    Router::new()
        .route("/", post(rpc_handler))
        .route("/health", get(health_check))
        .layer(cors)
        .with_state(state)
}

async fn rpc_handler(
    State(state): State<AppState>,
    Json(payload): Json<JsonRpcPayload>,
) -> Response {
    match payload {
        JsonRpcPayload::Single(req) => {
            let res = process_rpc_call(&state, req).await;
            (StatusCode::OK, [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))], Json(res)).into_response()
        }
        JsonRpcPayload::Batch(requests) => {
            let mut responses = Vec::with_capacity(requests.len());
            for req in requests {
                responses.push(process_rpc_call(&state, req).await);
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))], Json(responses)).into_response()
        }
    }
}

async fn process_rpc_call(state: &AppState, req: JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        "eth_call" => match state.executor.eth_call(req.params).await {
            Ok(result) => JsonRpcResponse::success(req.id, result),
            Err(err) => JsonRpcResponse::error(req.id, err),
        },
        "eth_estimateGas" => match state.executor.eth_estimate_gas(req.params).await {
            Ok(result) => JsonRpcResponse::success(req.id, result),
            Err(err) => JsonRpcResponse::error(req.id, err),
        },
        _ => forward_upstream_raw(state, req).await,
    }
}

async fn forward_upstream_raw(state: &AppState, req: JsonRpcRequest) -> JsonRpcResponse {
    let req_id = req.id.clone();
    let upstream_resp = state
        .http_client
        .post(&state.upstream_url)
        .json(&req)
        .send()
        .await;

    match upstream_resp {
        Ok(resp) => match resp.json::<Value>().await {
            Ok(val) => {
                if let Some(res) = val.get("result") {
                    JsonRpcResponse::success(req_id, res.clone())
                } else if let Some(err_val) = val.get("error") {
                    let err: JsonRpcError = serde_json::from_value(err_val.clone()).unwrap_or_else(|_| {
                        JsonRpcError::internal_error("Failed to parse upstream error")
                    });
                    JsonRpcResponse::error(req_id, err)
                } else {
                    JsonRpcResponse::success(req_id, val)
                }
            }
            Err(e) => JsonRpcResponse::error(
                req_id,
                JsonRpcError::internal_error(format!("Malformed upstream response: {e}")),
            ),
        },
        Err(e) => JsonRpcResponse::error(
            req_id,
            JsonRpcError::internal_error(format!("Upstream RPC connection failed: {e}")),
        ),
    }
}

async fn health_check() -> &'static str {
    "Phalanx Daemon v0.1.0 - OK"
}