use alloy_primitives::U256;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use phalanx_accumulator::buffer::AlignedSlotAccumulator;
use phalanx_accumulator::reduction::{reduce_16, reduce_scalar_fallback};

fn bench_reduction_kernels(c: &mut Criterion) {
    let mut group = c.benchmark_group("barrier_kernels");
    group.sample_size(100);

    let accumulators = [(); 16].map(|_| AlignedSlotAccumulator::new());
    for i in 0..16 {
        unsafe {
            accumulators[i].record_delta(U256::from((i as u64) * 123_456_789 + 42), 21_000);
        }
    }

    group.bench_function("reduce_16_hardware", |b| {
        b.iter(|| {
            let res = reduce_16(black_box(&accumulators));
            black_box(res)
        });
    });

    group.bench_function("reduce_scalar_fallback", |b| {
        b.iter(|| {
            let res = reduce_scalar_fallback(black_box(&accumulators));
            black_box(res)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_reduction_kernels);
criterion_main!(benches);