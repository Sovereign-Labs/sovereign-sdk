#![allow(clippy::float_arithmetic)]

use std::env;

use criterion::{criterion_group, criterion_main, Criterion};
use sov_address::MultiAddressEvm;
use sov_benchmarks::node::{assert_batch_receipts, generate_transfers, prefill_state};
use sov_benchmarks::{setup_with_runner, BenchSpec, NomtBenchSpec};
use sov_mock_da::MockDaSpec;
use sov_modules_api::Spec;
use sov_test_utils::storage::{
    ForklessStorageManager, SimpleNomtStorageManager, SimpleStorageManager,
};
use sov_test_utils::MockZkvm;

fn run_spec<S, Sm>(c: &mut Criterion, name: &str, senders_count: u64, bench_after_blocks: u64)
where
    Sm: ForklessStorageManager,
    S: Spec<
        InnerZkvm = MockZkvm,
        OuterZkvm = MockZkvm,
        Da = MockDaSpec,
        Address = MultiAddressEvm,
        Storage = Sm::Storage,
    >,
{
    let (mut runner, roles) =
        setup_with_runner::<S, MockZkvm, Sm>(senders_count, Default::default());

    let token_id = prefill_state(&roles, &mut runner);
    let bench_messages = generate_transfers(bench_after_blocks, token_id, &roles, &mut runner);

    for message in bench_messages {
        let apply_slot_output = runner.execute(message);
        assert_batch_receipts(&apply_slot_output.0.batch_receipts);
    }

    c.bench_function(&format!("rollup main stf loop: {name}"), |b| {
        b.iter(|| {
            let bench_messages = generate_transfers(1, token_id, &roles, &mut runner)
                .pop()
                .unwrap();
            let apply_slot_output = runner.execute(bench_messages);
            assert_batch_receipts(&apply_slot_output.0.batch_receipts);
        });
    });
}

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

    run_spec::<BenchSpec<MockZkvm>, SimpleStorageManager<_>>(
        c,
        "jmt",
        senders_count,
        bench_after_blocks,
    );
    run_spec::<NomtBenchSpec, SimpleNomtStorageManager<_>>(
        c,
        "nomt",
        senders_count,
        bench_after_blocks,
    );
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
