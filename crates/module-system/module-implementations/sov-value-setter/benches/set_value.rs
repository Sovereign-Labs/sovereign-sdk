use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use serde::Deserialize;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{
    generate_zk_runtime, AsUser, BatchTestCase, BatchType, TestSpec, TestUser, TransactionType,
};
use sov_value_setter::CallMessage;

generate_zk_runtime!(BenchRuntime <= value_setter: ValueSetter<S>);

type S = TestSpec;
type RT = BenchRuntime<S>;

// Large N so the per-call signal dominates the fixed per-slot overhead; the fit
// slope (equivalently a large/small difference) then cancels the overhead cleanly.
const BATCH_SIZES: &[usize] = &[4096, 16384, 65536];

// Policy: 1 ns wall-clock = 1 gas. Applied uniformly across all native-calibrated
// constants; rescale by adjusting INITIAL_GAS_LIMIT if the absolute scale shifts.
const GAS_PER_NS: f64 = 1.0;

const BENCH_GROUP: &str = "value_setter_set_value";

fn build_runner() -> (TestRunner<RT, S>, TestUser<S>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(1);
    let admin = genesis_config.additional_accounts()[0].clone();
    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        sov_value_setter::ValueSetterConfig {
            admin: admin.address(),
        },
    );
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default());
    (runner, admin)
}

fn build_batch(admin: &TestUser<S>, n: usize) -> BatchType<RT, S> {
    // Asymmetric, max-width borsh varint; defeats constant folding.
    let value: u32 = 0x12345678;
    let txs: Vec<TransactionType<RT, S>> = (0..n)
        .map(|_| {
            admin.create_plain_message::<RT, ValueSetter<S>>(CallMessage::SetValue {
                value,
                gas: None,
            })
        })
        .collect();
    BatchType(txs)
}

fn bench_set_value(c: &mut Criterion) {
    let mut group = c.benchmark_group(BENCH_GROUP);
    group.sample_size(50);
    for &n in BATCH_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let (runner, admin) = build_runner();
                    let batch = build_batch(&admin, n);
                    (runner, batch)
                },
                |(mut runner, batch)| {
                    runner.execute_batch(BatchTestCase {
                        input: batch,
                        assert: Box::new(|_, _| {}),
                    });
                    black_box(runner);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();

    if let Err(e) = report_fit_and_suggested_constant() {
        eprintln!("warn: could not compute fit / constants suggestion: {e}");
    }
}

#[derive(Deserialize)]
struct Point {
    point_estimate: f64,
}

#[derive(Deserialize)]
struct Estimates {
    mean: Point,
}

fn target_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    // CARGO_MANIFEST_DIR is the crate root; walk up to workspace root.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .map(|p| p.join("target"))
        .unwrap_or_else(|| manifest.join("target"))
}

fn read_mean_ns(n: usize) -> anyhow::Result<f64> {
    let path = target_dir()
        .join("criterion")
        .join(BENCH_GROUP)
        .join(n.to_string())
        .join("new/estimates.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let est: Estimates = serde_json::from_str(&raw)?;
    Ok(est.mean.point_estimate)
}

fn fit_ns_per_call(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    let sx: f64 = points.iter().map(|(x, _)| *x).sum();
    let sy: f64 = points.iter().map(|(_, y)| *y).sum();
    let sxx: f64 = points.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    let intercept = (sy - slope * sx) / n;
    (slope, intercept)
}

fn ns_to_gas(ns: f64) -> u64 {
    let gas = (ns * GAS_PER_NS).ceil() as u64;
    gas.max(1)
}

fn report_fit_and_suggested_constant() -> anyhow::Result<()> {
    let mut points = Vec::with_capacity(BATCH_SIZES.len());
    println!(
        "\n[fit] reading criterion estimates from {}",
        target_dir().join("criterion").join(BENCH_GROUP).display()
    );
    for &n in BATCH_SIZES {
        let mean_ns = read_mean_ns(n)?;
        println!("  N={n:<4} mean ns/slot = {mean_ns:.0}");
        points.push((n as f64, mean_ns));
    }
    let (ns_per_call, slot_overhead) = fit_ns_per_call(&points);
    let gas_per_call = ns_to_gas(ns_per_call);

    println!("\n=== value_setter::set_value calibration ===");
    println!("  ns per call          = {ns_per_call:.2}");
    println!("  slot overhead (ns)   = {slot_overhead:.2}  (informational, not charged per call)");
    println!("  GAS_PER_NS           = {GAS_PER_NS}");

    println!("\n=== suggested constants.toml entry ===");
    println!("VALUE_SETTER_SET_VALUE_CALL_GAS = [{gas_per_call}, 0]");
    Ok(())
}

criterion_group!(benches, bench_set_value);
criterion_main!(benches);
