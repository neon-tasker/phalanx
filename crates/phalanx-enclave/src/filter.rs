//! High-throughput partition filters and recipient account verification.

use alloy_primitives::{Address, B256};
use core::sync::atomic::{AtomicU64, Ordering};

/// Canonical Keccak256 hash of empty contract code.
pub const KECCAK_EMPTY: B256 = B256::new([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03,
    0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85,
    0xa4, 0x70,
]);

/// Verifies that an account is an EOA with no code.
#[inline(always)]
pub fn is_eoa_recipient(code_hash: &B256, code_len: usize) -> bool {
    code_len == 0 && (*code_hash == KECCAK_EMPTY || *code_hash == B256::ZERO)
}

/// 64K-bit lock-free atomic bitset evaluating recipient partition exclusivity in < 15 ns.
pub struct LockFreeDisjointFilter {
    buckets: [AtomicU64; 1024],
}

impl Default for LockFreeDisjointFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl LockFreeDisjointFilter {
    /// Instantiates an empty 64K-bit filter.
    pub fn new() -> Self {
        Self {
            buckets: [(); 1024].map(|_| AtomicU64::new(0)),
        }
    }

    #[inline(always)]
    fn hash_address(address: &Address) -> usize {
        let bytes = address.as_slice();
        let mut hash: u64 = 0xcbf29ce484222325;
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        (hash ^ (hash >> 32)) as usize & 0xFFFF
    }

    /// Attempts to acquire partition bit. Returns true if acquired, false on collision.
    #[inline(always)]
    pub fn try_acquire(&self, address: &Address) -> bool {
        let bit_index = Self::hash_address(address);
        let bucket_idx = bit_index / 64;
        let bit_mask = 1u64 << (bit_index % 64);

        let prev = self.buckets[bucket_idx].fetch_or(bit_mask, Ordering::AcqRel);
        (prev & bit_mask) == 0
    }

    /// Resets all buckets.
    pub fn reset(&self) {
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Release);
        }
    }
}