//! 128-byte hardware cache-line aligned accumulators eliminating cross-core false sharing.

use alloy_primitives::U256;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

/// Thread-local storage delta accumulator strictly pinned to 128-byte hardware boundaries.
#[repr(align(128))]
pub struct AlignedSlotAccumulator {
    delta: UnsafeCell<U256>,
    write_count: UnsafeCell<u64>,
    gas_accumulated: UnsafeCell<u64>,
    flags: AtomicU32,
    _pad: [u8; 76],
}

const _: () = {
    assert!(core::mem::size_of::<AlignedSlotAccumulator>() == 128);
    assert!(core::mem::align_of::<AlignedSlotAccumulator>() == 128);
};

unsafe impl Send for AlignedSlotAccumulator {}
unsafe impl Sync for AlignedSlotAccumulator {}

impl Default for AlignedSlotAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl AlignedSlotAccumulator {
    /// Flag indicating mid-flight transaction ejection.
    pub const FLAG_EJECTED: u32 = 1 << 0;

    /// Creates an empty accumulator.
    pub const fn new() -> Self {
        Self {
            delta: UnsafeCell::new(U256::ZERO),
            write_count: UnsafeCell::new(0),
            gas_accumulated: UnsafeCell::new(0),
            flags: AtomicU32::new(0),
            _pad: [0u8; 76],
        }
    }

    /// Records an additive mutation delta into thread-isolated storage.
    #[inline(always)]
    pub unsafe fn record_delta(&self, val: U256, gas: u64) {
        let delta_ptr = self.delta.get();
        *delta_ptr = (*delta_ptr).wrapping_add(val);
        *self.write_count.get() += 1;
        *self.gas_accumulated.get() += gas;
    }

    /// Unwinds an admitted delta if an ejection occurs mid-flight.
    #[inline(always)]
    pub unsafe fn unwind_delta(&self, val: U256, gas: u64) {
        let delta_ptr = self.delta.get();
        *delta_ptr = (*delta_ptr).wrapping_sub(val);
        *self.write_count.get() = (*self.write_count.get()).saturating_sub(1);
        *self.gas_accumulated.get() = (*self.gas_accumulated.get()).saturating_sub(gas);
    }

    /// Sets thread execution flags.
    pub fn raise_flag(&self, flag: u32) {
        self.flags.fetch_or(flag, Ordering::Release);
    }

    /// Checks if a specific flag is set.
    pub fn has_flag(&self, flag: u32) -> bool {
        (self.flags.load(Ordering::Acquire) & flag) != 0
    }

    /// Reads accumulated delta.
    #[inline(always)]
    pub unsafe fn read_delta(&self) -> U256 {
        *self.delta.get()
    }

    /// Reads accumulated gas.
    #[inline(always)]
    pub unsafe fn read_gas(&self) -> u64 {
        *self.gas_accumulated.get()
    }

    /// Reads write count.
    #[inline(always)]
    pub unsafe fn read_write_count(&self) -> u64 {
        *self.write_count.get()
    }

    /// Resets all values.
    pub unsafe fn reset(&self) {
        *self.delta.get() = U256::ZERO;
        *self.write_count.get() = 0;
        *self.gas_accumulated.get() = 0;
        self.flags.store(0, Ordering::Release);
    }
}

/// Heap-allocated arena of aligned accumulators sized to execution threads.
pub struct HotspotAccumulatorArena {
    accumulators: Box<[AlignedSlotAccumulator]>,
    #[allow(dead_code)]
    thread_count: usize,
}

impl HotspotAccumulatorArena {
    /// Allocates an arena with `thread_count` elements.
    pub fn new(thread_count: usize) -> Self {
        assert!(thread_count > 0);
        let mut vec = Vec::with_capacity(thread_count);
        for _ in 0..thread_count {
            vec.push(AlignedSlotAccumulator::new());
        }
        Self {
            accumulators: vec.into_boxed_slice(),
            thread_count,
        }
    }

    /// Obtains an immutable reference to the designated thread-local accumulator.
    #[inline(always)]
    pub fn get(&self, thread_id: usize) -> &AlignedSlotAccumulator {
        &self.accumulators[thread_id]
    }

    /// Returns the raw slice of all accumulators.
    #[inline(always)]
    pub fn as_slice(&self) -> &[AlignedSlotAccumulator] {
        &self.accumulators
    }

    /// Resets all accumulators across the arena.
    pub fn reset_all(&self) {
        for acc in self.accumulators.iter() {
            unsafe { acc.reset() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accumulator_layout() {
        let acc = AlignedSlotAccumulator::new();
        unsafe {
            acc.record_delta(alloy_primitives::U256::from(100u64), 500);
            assert_eq!(acc.read_delta(), alloy_primitives::U256::from(100u64));
            assert_eq!(acc.read_gas(), 500);
            assert_eq!(acc.read_write_count(), 1);
        }
    }
}