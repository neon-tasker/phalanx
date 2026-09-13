use alloy_primitives::{Address, Bytes, B256, U256};
use moka::sync::Cache;
use reqwest::Client;
use revm::primitives::{AccountInfo, Bytecode, KECCAK_EMPTY};
use revm::{Database, DatabaseRef};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone, thiserror::Error)]
pub enum DatabaseError {
    #[error("Upstream RPC transport error: {0}")]
    Transport(String),
    #[error("Missing slot trapping triggered")]
    MissingSlotTrapped,
}

#[derive(Clone)]
pub struct PipelinedJitDb {
    pub rpc_pool: Vec<String>,
    pub current_idx: Arc<AtomicUsize>,
    pub http_client: Client,
    pub account_cache: Cache<Address, AccountInfo>,
    pub storage_cache: Cache<(Address, U256), U256>,
    pub block_hash_cache: Cache<u64, B256>,
}

impl PipelinedJitDb {
    pub fn new(primary_url: String, _client: Client) -> Self {
        let pool = vec![
            primary_url,
            "https://ethereum-rpc.publicnode.com".to_string(),
            "https://1rpc.io/eth".to_string(),
            "https://eth.merkle.io".to_string(),
        ];

        let http_client = Client::builder()
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap_or_default();

        Self {
            rpc_pool: pool,
            current_idx: Arc::new(AtomicUsize::new(0)),
            http_client,
            account_cache: Cache::builder().max_capacity(100_000).build(),
            storage_cache: Cache::builder().max_capacity(500_000).build(),
            block_hash_cache: Cache::builder().max_capacity(1_000).build(),
        }
    }

    async fn dispatch_rpc(&self, body: Value) -> Result<Value, DatabaseError> {
        let pool_len = self.rpc_pool.len();
        let start = self.current_idx.load(Ordering::Relaxed);

        for offset in 0..pool_len {
            let active_idx = (start + offset) % pool_len;
            let target_url = &self.rpc_pool[active_idx];

            let res = self.http_client
                .post(target_url)
                .json(&body)
                .send()
                .await;

            match res {
                Ok(response) => {
                    if response.status().is_success() {
                        if let Ok(json_res) = response.json::<Value>().await {
                            let is_rate_limit = json_res.get("error").and_then(|e| e.get("message")).map_or(false, |m| {
                                let msg = m.as_str().unwrap_or("").to_lowercase();
                                msg.contains("rate") || msg.contains("limit") || msg.contains("429") || msg.contains("throughput")
                            });

                            if !is_rate_limit {
                                if offset > 0 {
                                    self.current_idx.store(active_idx, Ordering::Relaxed);
                                }
                                return Ok(json_res);
                            }
                        }
                    }
                    warn!("RPC {} throttled/busy. Switching...", target_url);
                }
                Err(err) => {
                    warn!("RPC {} network hitch: {}. Switching...", target_url, err);
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        Err(DatabaseError::Transport("All fallback RPC nodes busy.".to_string()))
    }

    pub fn create_session(&self, dry_run: bool) -> SessionDb {
        SessionDb {
            shared: self.clone(),
            dry_run: Arc::new(AtomicBool::new(dry_run)),
            trapped_accounts: Arc::new(Mutex::new(HashSet::new())),
            trapped_slots: Arc::new(Mutex::new(HashSet::new())),
            ephemeral_storage: Arc::new(Mutex::new(HashMap::new())),
            ephemeral_accounts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn hydrate_batch(
        &self,
        accounts: &[Address],
        slots: &[(Address, U256)],
        block_param: &str,
    ) -> Result<(), DatabaseError> {
        let mut batch = Vec::new();
        let mut id = 0;

        for acc in accounts {
            if self.account_cache.get(acc).is_none() {
                let addr_hex = format!("{acc:#x}");
                batch.push(json!({"jsonrpc": "2.0", "id": id, "method": "eth_getBalance", "params": [addr_hex, block_param]}));
                id += 1;
                batch.push(json!({"jsonrpc": "2.0", "id": id, "method": "eth_getTransactionCount", "params": [addr_hex, block_param]}));
                id += 1;
                batch.push(json!({"jsonrpc": "2.0", "id": id, "method": "eth_getCode", "params": [addr_hex, block_param]}));
                id += 1;
            }
        }

        for (addr, slot) in slots {
            if self.storage_cache.get(&(*addr, *slot)).is_none() {
                batch.push(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "eth_getStorageAt",
                    "params": [format!("{addr:#x}"), format!("{slot:#x}"), block_param]
                }));
                id += 1;
            }
        }

        if batch.is_empty() {
            return Ok(());
        }

        let json_res = self.dispatch_rpc(Value::Array(batch)).await?;
        if let Some(arr) = json_res.as_array() {
            let mut acc_idx = 0;
            for acc in accounts {
                if self.account_cache.get(acc).is_none() {
                    let bal_hex = arr.get(acc_idx).and_then(|r| r.get("result")).and_then(|v| v.as_str()).unwrap_or("0x0");
                    let nonce_hex = arr.get(acc_idx + 1).and_then(|r| r.get("result")).and_then(|v| v.as_str()).unwrap_or("0x0");
                    let code_hex = arr.get(acc_idx + 2).and_then(|r| r.get("result")).and_then(|v| v.as_str()).unwrap_or("0x");

                    let balance = U256::from_str_radix(bal_hex.trim_start_matches("0x"), 16).unwrap_or(U256::ZERO);
                    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16).unwrap_or(0);
                    let code_clean = code_hex.trim_start_matches("0x");
                    let raw_bytes = if let Ok(b) = alloy_primitives::hex::decode(code_clean) { b } else { Vec::new() };

                    let bytecode = if raw_bytes.is_empty() {
                        None
                    } else {
                        Some(Bytecode::new_raw(Bytes::from(raw_bytes)))
                    };
                    let code_hash = bytecode.as_ref().map(|b| b.hash_slow()).unwrap_or(KECCAK_EMPTY);

                    self.account_cache.insert(*acc, AccountInfo { balance, nonce, code_hash, code: bytecode });
                    acc_idx += 3;
                }
            }

            let mut slot_cursor = acc_idx;
            for (addr, slot) in slots {
                if self.storage_cache.get(&(*addr, *slot)).is_none() {
                    if let Some(val_hex) = arr.get(slot_cursor).and_then(|r| r.get("result")).and_then(|v| v.as_str()) {
                        let val = U256::from_str_radix(val_hex.trim_start_matches("0x"), 16).unwrap_or(U256::ZERO);
                        self.storage_cache.insert((*addr, *slot), val);
                    }
                    slot_cursor += 1;
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct SessionDb {
    pub shared: PipelinedJitDb,
    pub dry_run: Arc<AtomicBool>,
    pub trapped_accounts: Arc<Mutex<HashSet<Address>>>,
    pub trapped_slots: Arc<Mutex<HashSet<(Address, U256)>>>,
    pub ephemeral_storage: Arc<Mutex<HashMap<(Address, U256), U256>>>,
    pub ephemeral_accounts: Arc<Mutex<HashMap<Address, AccountInfo>>>,
}

impl SessionDb {
    pub fn is_dry_run(&self) -> bool {
        self.dry_run.load(Ordering::Relaxed)
    }
}

impl DatabaseRef for SessionDb {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if let Some(info) = self.ephemeral_accounts.lock().unwrap().get(&address) {
            return Ok(Some(info.clone()));
        }
        if let Some(info) = self.shared.account_cache.get(&address) {
            return Ok(Some(info));
        }
        if self.is_dry_run() {
            self.trapped_accounts.lock().unwrap().insert(address);
            return Ok(Some(AccountInfo::default()));
        }
        Ok(Some(AccountInfo::default()))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::default())
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if let Some(val) = self.ephemeral_storage.lock().unwrap().get(&(address, index)) {
            return Ok(*val);
        }
        if let Some(val) = self.shared.storage_cache.get(&(address, index)) {
            return Ok(val);
        }
        if self.is_dry_run() {
            self.trapped_slots.lock().unwrap().insert((address, index));
            return Ok(U256::ZERO);
        }
        Ok(U256::ZERO)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        if let Some(hash) = self.shared.block_hash_cache.get(&number) {
            return Ok(hash);
        }
        Ok(B256::ZERO)
    }
}

impl Database for SessionDb {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        DatabaseRef::basic_ref(self, address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        DatabaseRef::code_by_hash_ref(self, code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        DatabaseRef::storage_ref(self, address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        DatabaseRef::block_hash_ref(self, number)
    }
}