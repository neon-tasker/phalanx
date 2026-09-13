use alloy_primitives::{address, Address, TxKind, U256};
use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};
use phalanx_core::{MutationOp, StorageTarget};
use phalanx_pipeline::dispatcher::{CommutativeTxEnvelope, DualPathDispatcher};
use revm::primitives::TxEnv;
use std::sync::atomic::{AtomicU32, Ordering};

const BATCH_SIZE: usize = 50_000;

fn allocate_benchmark_statuses(count: usize) -> &'static [AtomicU32] {
    let vec: Vec<AtomicU32> = (0..count).map(|_| AtomicU32::new(0)).collect();
    Box::leak(vec.into_boxed_slice())
}

fn generate_hotspot_mint_workload(
    count: usize,
    target: StorageTarget,
    statuses: &'static [AtomicU32],
) -> Vec<CommutativeTxEnvelope> {
    let mut txs = Vec::with_capacity(count);
    for i in 0..count {
        let mut recipient_bytes = [0u8; 20];
        recipient_bytes[16..20].copy_from_slice(&(i as u32 + 1).to_be_bytes());
        recipient_bytes[0] = 0xBB;

        let mut tx = TxEnv::default();
        tx.caller = Address::from(recipient_bytes);
        tx.transact_to = TxKind::Call(target.address);
        tx.gas_limit = 100_000;

        txs.push(CommutativeTxEnvelope {
            tx,
            tx_index: i,
            target,
            op: MutationOp::Add,
            delta: U256::from(1u64),
            recipient: Address::from(recipient_bytes),
            base_gas: 21_000,
            status: &statuses[i],
        });
    }
    txs
}

fn bench_hotspot_mint_scaling(c: &mut Criterion) {
    let target = StorageTarget::new(
        address!("34d85c9CDeB23FA97cb08333b511ac86E1C4E258"),
        U256::from(0x07),
    );
    let initial_val = U256::from(1_000_000u64);
    let statuses = allocate_benchmark_statuses(BATCH_SIZE);
    let base_workload = generate_hotspot_mint_workload(BATCH_SIZE, target, statuses);

    let mut group = c.benchmark_group("hotspot_mint_50k");
    group.sample_size(10);
    group.throughput(Throughput::Elements(BATCH_SIZE as u64));

    // Baseline: Sequential execution
    group.bench_function("baseline_sequential", |b| {
        b.iter_batched(
            || base_workload.clone(),
            |batch| {
                let mut current_slot = initial_val;
                let mut cumulative_gas = 0u64;

                for tx in batch {
                    current_slot = current_slot.wrapping_add(tx.delta);
                    let gas = if tx.tx_index == 0 {
                        tx.base_gas + 9700
                    } else {
                        tx.base_gas + 3100
                    };
                    cumulative_gas += gas;
                }

                black_box((current_slot, cumulative_gas))
            },
            BatchSize::LargeInput,
        );
    });

    // Multi-core parallel execution across 1, 4, 8, 16 threads
    for &threads in &[1, 4, 8, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let dispatcher = DualPathDispatcher::new(threads);

        group.bench_with_input(
            BenchmarkId::new("phalanx_parallel", threads),
            &threads,
            |b, _| {
                b.iter_batched(
                    || {
                        for s in statuses.iter() {
                            s.store(0, Ordering::Relaxed);
                        }
                        base_workload.clone()
                    },
                    |batch| {
                        let res = pool.install(|| {
                            dispatcher.dispatch_batch(batch, initial_val).unwrap()
                        });
                        black_box(res)
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_hotspot_mint_scaling);
criterion_main!(benches);