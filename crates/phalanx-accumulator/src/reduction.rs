//! Silicon-level barrier reduction kernels for summing 16 thread accumulators in < 20 ns.

use crate::buffer::AlignedSlotAccumulator;
use alloy_primitives::U256;

/// Sums 16 thread accumulators into a single canonical U256.
#[inline(always)]
pub fn reduce_16(accumulators: &[AlignedSlotAccumulator; 16]) -> (U256, bool) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("adx") {
            return unsafe { reduce_16_x86_adx(accumulators) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { reduce_16_arm64_adcs(accumulators) };
    }

    reduce_scalar_fallback(accumulators)
}

/// x86_64 dual carry-chain assembly kernel (ADCX/ADOX) utilizing memory operands
/// to strictly bound register allocation to 9 GPRs (preventing Windows MSVC register exhaustion).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "adx")]
pub unsafe fn reduce_16_x86_adx(accumulators: &[AlignedSlotAccumulator; 16]) -> (U256, bool) {
    let base_ptr = accumulators.as_ptr() as *const u8;
    let p0 = base_ptr as *const u64;
    let mut l0 = *p0;
    let mut l1 = *p0.add(1);
    let mut l2 = *p0.add(2);
    let mut l3 = *p0.add(3);
    let mut ovf_accum: u64 = 0;

    let pairs: [(usize, usize); 7] = [
        (1, 2), (3, 4), (5, 6), (7, 8), (9, 10), (11, 12), (13, 14),
    ];

    for (idx_a, idx_b) in pairs {
        let ptr_a = base_ptr.add(idx_a * 128) as *const u64;
        let ptr_b = base_ptr.add(idx_b * 128) as *const u64;
        let mut carry_a: u64 = 0;
        let mut carry_b: u64 = 0;

        core::arch::asm!(
            "xor {scratch:e}, {scratch:e}",
            "adcx {r0}, qword ptr [{ptr_a}]",
            "adox {r0}, qword ptr [{ptr_b}]",
            "adcx {r1}, qword ptr [{ptr_a} + 8]",
            "adox {r1}, qword ptr [{ptr_b} + 8]",
            "adcx {r2}, qword ptr [{ptr_a} + 16]",
            "adox {r2}, qword ptr [{ptr_b} + 16]",
            "adcx {r3}, qword ptr [{ptr_a} + 24]",
            "adox {r3}, qword ptr [{ptr_b} + 24]",
            "adcx {ca}, {scratch}",
            "adox {cb}, {scratch}",
            scratch = out(reg) _,
            r0 = inout(reg) l0,
            r1 = inout(reg) l1,
            r2 = inout(reg) l2,
            r3 = inout(reg) l3,
            ptr_a = in(reg) ptr_a,
            ptr_b = in(reg) ptr_b,
            ca = inout(reg) carry_a,
            cb = inout(reg) carry_b,
            options(nostack)
        );

        ovf_accum |= carry_a | carry_b;
    }

    let ptr_15 = base_ptr.add(15 * 128) as *const u64;
    let mut tail_carry: u64 = 0;

    core::arch::asm!(
        "xor {scratch:e}, {scratch:e}",
        "adcx {r0}, qword ptr [{ptr_15}]",
        "adcx {r1}, qword ptr [{ptr_15} + 8]",
        "adcx {r2}, qword ptr [{ptr_15} + 16]",
        "adcx {r3}, qword ptr [{ptr_15} + 24]",
        "adcx {tc}, {scratch}",
        scratch = out(reg) _,
        r0 = inout(reg) l0,
        r1 = inout(reg) l1,
        r2 = inout(reg) l2,
        r3 = inout(reg) l3,
        ptr_15 = in(reg) ptr_15,
        tc = inout(reg) tail_carry,
        options(nostack)
    );

    ovf_accum |= tail_carry;

    (U256::from_limbs([l0, l1, l2, l3]), ovf_accum != 0)
}

/// ARM64 unrolled assembly kernel (ADDS/ADCS).
#[cfg(target_arch = "aarch64")]
pub unsafe fn reduce_16_arm64_adcs(accumulators: &[AlignedSlotAccumulator; 16]) -> (U256, bool) {
    let mut l0: u64; let mut l1: u64; let mut l2: u64; let mut l3: u64;
    let mut ovf_accum: u64 = 0;

    let base_ptr = accumulators.as_ptr() as *const u8;
    let p0 = base_ptr as *const u64;
    l0 = *p0; l1 = *p0.add(1); l2 = *p0.add(2); l3 = *p0.add(3);

    for i in 1..16 {
        let ptr_i = base_ptr.add(i * 128) as *const u64;
        let a0 = *ptr_i; let a1 = *ptr_i.add(1); let a2 = *ptr_i.add(2); let a3 = *ptr_i.add(3);
        let mut carry: u64;

        core::arch::asm!(
            "adds {r0}, {r0}, {a0}",
            "adcs {r1}, {r1}, {a1}",
            "adcs {r2}, {r2}, {a2}",
            "adcs {r3}, {r3}, {a3}",
            "cset {c}, cs",
            r0 = inout(reg) l0,
            r1 = inout(reg) l1,
            r2 = inout(reg) l2,
            r3 = inout(reg) l3,
            a0 = in(reg) a0,
            a1 = in(reg) a1,
            a2 = in(reg) a2,
            a3 = in(reg) a3,
            c = out(reg) carry,
            options(nostack)
        );

        ovf_accum |= carry;
    }

    (U256::from_limbs([l0, l1, l2, l3]), ovf_accum != 0)
}

/// Portable multi-limb scalar fallback with u128 carry propagation.
pub fn reduce_scalar_fallback(accumulators: &[AlignedSlotAccumulator]) -> (U256, bool) {
    let mut l0: u64 = 0; let mut l1: u64 = 0; let mut l2: u64 = 0; let mut l3: u64 = 0;
    let mut global_overflow = false;

    for acc in accumulators.iter() {
        let limbs = unsafe { acc.read_delta() }.into_limbs();

        let sum0 = (l0 as u128) + (limbs[0] as u128);
        l0 = sum0 as u64;
        let c0 = sum0 >> 64;

        let sum1 = (l1 as u128) + (limbs[1] as u128) + c0;
        l1 = sum1 as u64;
        let c1 = sum1 >> 64;

        let sum2 = (l2 as u128) + (limbs[2] as u128) + c1;
        l2 = sum2 as u64;
        let c2 = sum2 >> 64;

        let sum3 = (l3 as u128) + (limbs[3] as u128) + c2;
        l3 = sum3 as u64;
        let c3 = sum3 >> 64;

        if c3 != 0 {
            global_overflow = true;
        }
    }

    (U256::from_limbs([l0, l1, l2, l3]), global_overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reduction_parity() {
        let arena = [(); 16].map(|_| AlignedSlotAccumulator::new());
        for i in 0..16 {
            unsafe {
                arena[i].record_delta(U256::from((i as u64) * 1000 + 1), 21000);
            }
        }

        let (res_scalar, ovf_scalar) = reduce_scalar_fallback(&arena);
        let (res_opt, ovf_opt) = reduce_16(&arena);

        assert_eq!(ovf_scalar, ovf_opt);
        assert_eq!(res_scalar, res_opt);
        assert!(!ovf_scalar);
        assert_eq!(res_scalar, U256::from(120016u64));
    }
}