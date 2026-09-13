//! In-memory revm simulation executor with pipelined Trap & Batch hydration.
use crate::engine::trap_db::PipelinedJitDb;
use crate::rpc::types::{CallObject, JsonRpcError};
use alloy_primitives::{Address, Bytes, TxKind, U256};
use revm::{
    primitives::{BlockEnv, CfgEnv, Env, ExecutionResult, Output, SpecId, TxEnv},
    Evm,
};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::debug;

pub struct SimulationExecutor {
    pub db: Arc<PipelinedJitDb>,
}

impl SimulationExecutor {
    pub fn new(db: Arc<PipelinedJitDb>) -> Self {
        Self { db }
    }

    pub async fn eth_call(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let (call_obj, block_param) = parse_call_params(params)?;
        let result_bytes = self.execute_pipelined(call_obj, &block_param).await?;
        let hex_data = alloy_primitives::hex::encode(&result_bytes);

        let formatted = if !hex_data.is_empty() && hex_data.len() < 64 {
            format!("0x{:0>64}", hex_data)
        } else if hex_data.is_empty() {
            "0x".to_string()
        } else {
            format!("0x{}", hex_data)
        };

        Ok(json!(formatted))
    }

    pub async fn eth_estimate_gas(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let (mut call_obj, block_param) = parse_call_params(params)?;
        if call_obj.gas.is_none() {
            call_obj.gas = Some(U256::from(30_000_000u64));
        }
        let gas_used = self.estimate_gas_pipelined(call_obj, &block_param).await?;
        let padded = ((gas_used as f64) * 1.15) as u64;
        let hex_encoded = format!("0x{:x}", padded.max(21_000));
        Ok(json!(hex_encoded))
    }

    async fn execute_pipelined(
        &self,
        call: CallObject,
        block_param: &str,
    ) -> Result<Bytes, JsonRpcError> {
        if let Some(to_addr) = call.to {
            if self.db.account_cache.get(&to_addr).is_none() {
                self.db
                    .hydrate_batch(&[to_addr], &[], block_param)
                    .await
                    .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
            }
        }

        let mut loop_count = 0;
        loop {
            loop_count += 1;
            let session = self.db.create_session(true);
            let dry_env = build_evm_env(&call);
            let _ = run_revm_instance(&session, dry_env);

            let missing_accs: Vec<Address> =
                session.trapped_accounts.lock().unwrap().drain().collect();
            let missing_slots: Vec<(Address, U256)> =
                session.trapped_slots.lock().unwrap().drain().collect();

            if (missing_accs.is_empty() && missing_slots.is_empty()) || loop_count >= 4 {
                break;
            }

            debug!(
                "Hydrating speculative missing slots (Pass {}): {} accounts, {} slots",
                loop_count,
                missing_accs.len(),
                missing_slots.len()
            );
            self.db
                .hydrate_batch(&missing_accs, &missing_slots, block_param)
                .await
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        }

        let session = self.db.create_session(false);
        session.dry_run.store(false, Ordering::Release);
        let final_env = build_evm_env(&call);
        let exec_result = run_revm_instance(&session, final_env)?;

        match exec_result {
            ExecutionResult::Success { output, .. } => match output {
                Output::Call(b) => Ok(b),
                Output::Create(b, _) => Ok(b),
            },
            ExecutionResult::Revert { output, .. } => {
                let hex_data = format!("0x{}", alloy_primitives::hex::encode(&output));
                Err(JsonRpcError::execution_reverted(hex_data))
            }
            ExecutionResult::Halt { reason, .. } => Err(JsonRpcError::internal_error(format!(
                "Execution halted: {:?}",
                reason
            ))),
        }
    }

    async fn estimate_gas_pipelined(
        &self,
        call: CallObject,
        block_param: &str,
    ) -> Result<u64, JsonRpcError> {
        if let Some(to_addr) = call.to {
            if self.db.account_cache.get(&to_addr).is_none() {
                let _ = self.db.hydrate_batch(&[to_addr], &[], block_param).await;
            }
        }

        let session = self.db.create_session(true);
        let dry_env = build_evm_env(&call);
        let _ = run_revm_instance(&session, dry_env);

        let missing_accs: Vec<Address> = session
            .trapped_accounts
            .lock()
            .unwrap()
            .drain()
            .collect();
        let missing_slots: Vec<(Address, U256)> =
            session.trapped_slots.lock().unwrap().drain().collect();

        if !missing_accs.is_empty() || !missing_slots.is_empty() {
            let _ = self
                .db
                .hydrate_batch(&missing_accs, &missing_slots, block_param)
                .await;
        }

        session.dry_run.store(false, Ordering::Release);
        let final_env = build_evm_env(&call);
        let exec_result = run_revm_instance(&session, final_env)?;

        match exec_result {
            ExecutionResult::Success { gas_used, .. } => Ok(gas_used),
            ExecutionResult::Revert { output, .. } => {
                let hex_data = format!("0x{}", alloy_primitives::hex::encode(&output));
                Err(JsonRpcError::execution_reverted(hex_data))
            }
            ExecutionResult::Halt { reason, .. } => Err(JsonRpcError::internal_error(format!(
                "Estimation halted: {:?}",
                reason
            ))),
        }
    }
}

fn parse_call_params(params: Option<Value>) -> Result<(CallObject, String), JsonRpcError> {
    let p = params.ok_or_else(|| JsonRpcError::invalid_params("Missing parameters"))?;
    match p {
        Value::Array(mut arr) => {
            if arr.is_empty() {
                return Err(JsonRpcError::invalid_params("Empty parameter array"));
            }
            let call_val = arr.remove(0);
            let call_obj: CallObject = serde_json::from_value(call_val)
                .map_err(|e| JsonRpcError::invalid_params(format!("Malformed call object: {e}")))?;

            let block_param = if !arr.is_empty() {
                arr.remove(0).as_str().unwrap_or("latest").to_string()
            } else {
                "latest".to_string()
            };

            Ok((call_obj, block_param))
        }
        _ => Err(JsonRpcError::invalid_params("Expected parameter array")),
    }
}

fn build_evm_env(call: &CallObject) -> Env {
    let mut cfg = CfgEnv::default();
    cfg.chain_id = call.chain_id.unwrap_or(1);

    let mut block = BlockEnv::default();
    block.gas_limit = U256::from(30_000_000);

    let mut tx = TxEnv::default();
    tx.caller = call.from.unwrap_or(Address::ZERO);
    tx.transact_to = match call.to {
        Some(to_addr) => TxKind::Call(to_addr),
        None => TxKind::Create,
    };
    tx.data = call.calldata();
    tx.value = call.value.unwrap_or(U256::ZERO);
    tx.gas_limit = call
        .gas
        .map(|g| g.saturating_to::<u64>())
        .unwrap_or(30_000_000);
    if let Some(gp) = call.gas_price {
        tx.gas_price = gp;
    }

    Env { cfg, block, tx }
}

fn run_revm_instance(
    session: &crate::engine::trap_db::SessionDb,
    env: Env,
) -> Result<ExecutionResult, JsonRpcError> {
    let mut evm = Evm::builder()
        .with_ref_db(session)
        .with_spec_id(SpecId::CANCUN)
        .with_env(Box::new(env))
        .build();

    evm.transact()
        .map(|res| res.result)
        .map_err(|e| JsonRpcError::internal_error(format!("revm execution failed: {:?}", e)))
}