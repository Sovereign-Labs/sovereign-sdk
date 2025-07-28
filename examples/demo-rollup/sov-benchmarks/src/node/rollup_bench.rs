#![allow(clippy::float_arithmetic)]

use std::env;

use criterion::{criterion_group, criterion_main, Criterion};
use sov_benchmarks::node::{assert_batch_receipts, generate_transfers, prefill_state};
use sov_benchmarks::setup_with_runner;

fn stf_apply_slot_bench(c: &mut Criterion) {
    let bench_after_blocks: u64 = env::var("SOV_BENCH_BLOCKS")
        .unwrap_or("100".to_string())
        .parse()
        .expect("SOV_BENCH_BLOCKS var should be a positive number");
    let senders_count = env::var("SOV_BENCH_TXNS_PER_BLOCK")
        .unwrap_or("1000".to_string())
        .parse()
        .expect("SOV_BENCH_TXS_PER_BLOCK var should be a positive number");

    tracing::info!(
        "Going to bench after {} blocks, with {} unique senders.",
        bench_after_blocks,
        senders_count
    );
    tracing::info!(
        "Meaning that when bench start there will be already {} transfers plus minting for each sender in the storage tree.",
        bench_after_blocks * senders_count
    );

    let (mut runner, roles) = setup_with_runner(senders_count, Default::default());

    let token_id = prefill_state(&roles, &mut runner);
    let bench_messages = generate_transfers(bench_after_blocks, token_id, &roles, &mut runner);

    for message in bench_messages {
        let apply_slot_output = runner.execute(message);
        assert_batch_receipts(&apply_slot_output.0.batch_receipts);
    }

    c.bench_function("rollup main stf loop", |b| {
        b.iter(|| {
            let bench_messages = generate_transfers(1, token_id, &roles, &mut runner)
                .pop()
                .unwrap();
            let apply_slot_output = runner.execute(bench_messages);
            assert_batch_receipts(&apply_slot_output.0.batch_receipts);
        });
    });
}

fn configure_criterion() -> Criterion {
    Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(20))
        .measurement_time(std::time::Duration::from_secs(80))
}

criterion_group! {
    name = benches;
    config = configure_criterion();
    targets = stf_apply_slot_bench
}
criterion_main!(benches);
