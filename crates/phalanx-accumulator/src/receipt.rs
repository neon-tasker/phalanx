//! Deterministic receipt construction, prefix-sum scanning, and SIMD LogsBloom aggregation.

use alloy_primitives::{Log, LogData};

/// Unprocessed execution telemetry emitted by a single transaction on a worker thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExecutionRecord {
    /// Canonical transaction index.
    pub tx_index: usize,
    /// Exact gas consumed.
    pub gas_used: u64,
    /// Raw un-indexed EVM logs emitted.
    pub logs: Vec<Log<LogData>>,
    /// Status code (true = 1 success, false = 0 revert).
    pub success: bool,
}

/// Consensus-compliant receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalReceipt {
    /// Canonical transaction index.
    pub tx_index: usize,
    /// Absolute cumulative gas used within the block up to this transaction.
    pub cumulative_gas_used: u64,
    /// 2048-bit LogsBloom filter.
    pub bloom: [u8; 256],
    /// Fully stamped logs.
    pub logs: Vec<Log<LogData>>,
    /// Execution status flag.
    pub status: bool,
}

/// Thread-isolated memory arena holding pre-allocated receipt vectors and local Bloom buffers.
#[repr(align(128))]
pub struct ThreadExecutionArena {
    /// Thread-local raw records.
    pub records: Vec<RawExecutionRecord>,
    /// Thread-local combined 2048-bit bloom filter.
    pub local_bloom: [u8; 256],
    /// Explicit padding to eliminate false sharing.
    _pad: [u8; 104],
}

impl Default for ThreadExecutionArena {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadExecutionArena {
    /// Allocates an empty thread execution arena.
    pub fn new() -> Self {
        Self {
            records: Vec::with_capacity(1024),
            local_bloom: [0u8; 256],
            _pad: [0u8; 104],
        }
    }

    /// Appends a raw execution record and merges log topics into local Bloom.
    pub fn push_record(&mut self, record: RawExecutionRecord) {
        for log in &record.logs {
            mempool_bloom_add_bytes(&mut self.local_bloom, log.address.as_slice());
            for topic in log.data.topics() {
                mempool_bloom_add_bytes(&mut self.local_bloom, topic.as_slice());
            }
        }
        self.records.push(record);
    }
}

/// Deterministic receipt pipeline.
pub struct DeterministicReceiptPipeline;

impl DeterministicReceiptPipeline {
    /// Aggregates receipts across thread arenas into a canonical ordered receipt vector.
    pub fn finalize(mut arenas: Vec<ThreadExecutionArena>) -> (Vec<CanonicalReceipt>, [u8; 256]) {
        let mut global_bloom = [0u8; 256];
        for arena in &arenas {
            merge_bloom_256bytes(&mut global_bloom, &arena.local_bloom);
        }

        let mut all_records = Vec::new();
        for mut arena in arenas.drain(..) {
            for record in arena.records.drain(..) {
                all_records.push(record);
            }
        }

        // Sort records strictly by canonical tx_index
        all_records.sort_by_key(|r| r.tx_index);

        let mut running_gas: u64 = 0;
        let mut canonical_receipts = Vec::with_capacity(all_records.len());
        for record in all_records {
            running_gas += record.gas_used;
            let mut stamped_receipt_bloom = [0u8; 256];
            for log in &record.logs {
                mempool_bloom_add_bytes(&mut stamped_receipt_bloom, log.address.as_slice());
                for topic in log.data.topics() {
                    mempool_bloom_add_bytes(&mut stamped_receipt_bloom, topic.as_slice());
                }
            }

            canonical_receipts.push(CanonicalReceipt {
                tx_index: record.tx_index,
                cumulative_gas_used: running_gas,
                bloom: stamped_receipt_bloom,
                logs: record.logs,
                status: record.success,
            });
        }

        (canonical_receipts, global_bloom)
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn merge_bloom_avx2(dest: &mut [u8; 256], src: &[u8; 256]) {
    use core::arch::x86_64::*;
    let mut d_ptr = dest.as_mut_ptr() as *mut __m256i;
    let mut s_ptr = src.as_ptr() as *const __m256i;
    for _ in 0..8 {
        let d = _mm256_loadu_si256(d_ptr as *const __m256i);
        let s = _mm256_loadu_si256(s_ptr);
        _mm256_storeu_si256(d_ptr, _mm256_or_si256(d, s));
        d_ptr = d_ptr.add(1);
        s_ptr = s_ptr.add(1);
    }
}

/// Merges 256-byte bloom vectors using AVX2 when present, otherwise uses portable byte-wise OR.
#[inline(always)]
pub fn merge_bloom_256bytes(dest: &mut [u8; 256], src: &[u8; 256]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            unsafe { merge_bloom_avx2(dest, src) };
            return;
        }
    }

    for (d, s) in dest.iter_mut().zip(src.iter()) {
        *d |= *s;
    }
}

#[inline(always)]
fn mempool_bloom_add_bytes(bloom: &mut [u8; 256], data: &[u8]) {
    let hash = alloy_primitives::keccak256(data);
    for i in [0, 2, 4] {
        let bit_index = ((hash[i] as usize) << 8 | (hash[i + 1] as usize)) & 0x07FF;
        let byte_index = 255 - (bit_index / 8);
        let bit_value = 1 << (bit_index % 8);
        bloom[byte_index] |= bit_value;
    }
}