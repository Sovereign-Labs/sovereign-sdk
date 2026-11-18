//! Node STF cost per transaction, as a low-noise metric for optimisation loops.
//!
//! One block of pre-signed gas-token transfers to fresh addresses is executed
//! `SOV_BENCH_SAMPLES` times from the same post-genesis state with `TestRunner::simulate`, which
//! discards the changes. One untimed warm-up execution fills the process-global signature
//! verification cache first, so auth reuses cached verifications as on a node behind its own
//! sequencer.
//!
//! The main figure times `StfBlueprint::apply_batches_in_user_space` through its tracing span:
//! the node's `blobs_processing` phase plus the per-slot begin/end block hooks. `slot_*` times
//! the whole `apply_slot`, which adds blob selection and the state root computation. Committing
//! to NOMT is never run.
//!
//! Output is one `stf_loop:` line. `receipts_digest` hashes the receipts (gas, outcomes, events)
//! and `state_root_digest` the post-block state root; both are identical in every sample (the bench
//! panics otherwise) and across runs, so a change in either means the STF output changed.
//!
//! Knobs: `SOV_BENCH_TXNS_PER_BLOCK` (default 10_000), `SOV_BENCH_SENDERS` (1_000),
//! `SOV_BENCH_SAMPLES` (15), `SOV_BENCH_METRICS=1` (enables the rollup's own metrics after the
//! warm-up and prints a second `stf_loop_metrics:` line; they slow the STF down, so don't compare
//! its main figure with metrics-off runs).

#![allow(clippy::float_arithmetic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use demo_stf::runtime::Runtime;
use sov_benchmarks::node::build_send_tx;
use sov_benchmarks::node::metrics::{start_metrics_collection, PhaseMetrics};
use sov_benchmarks::{setup, NomtBenchSpec};
use sov_rollup_interface::crypto::CredentialId;
use sov_shutdown::SecondaryShutdownController;
use sov_test_utils::runtime::sov_bank::config_gas_token_id;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::MockZkvm;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata};

type S = NomtBenchSpec;
type RT = Runtime<S>;
type Sm = SimpleStorageManager<
    sov_state::DefaultStorageSpec<
        <<S as sov_modules_api::Spec>::CryptoSpec as sov_modules_api::CryptoSpec>::Hasher,
    >,
>;

const TIMED_SPAN: &str = "apply_batches_in_user_space";

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| {
        v.parse()
            .unwrap_or_else(|_| panic!("{name} must be a positive integer"))
    })
}

/// Tracing subscriber that only enables [`TIMED_SPAN`] and records how long it was entered.
/// Every other span and event is disabled at its callsite, so it costs one span per slot.
#[derive(Default)]
struct SpanTimer {
    entered: std::sync::Mutex<Option<Instant>>,
    elapsed: std::sync::Mutex<Option<Duration>>,
}

impl SpanTimer {
    fn take(&self) -> Duration {
        self.elapsed
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| panic!("The `{TIMED_SPAN}` span was not entered; was it renamed?"))
    }
}

impl tracing::Subscriber for SpanTimer {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_span() && metadata.name() == TIMED_SPAN
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, _: &Id) {
        *self.entered.lock().unwrap() = Some(Instant::now());
    }

    fn exit(&self, _: &Id) {
        let entered = self.entered.lock().unwrap().take().expect("Entered first");
        *self.elapsed.lock().unwrap() = Some(entered.elapsed());
    }
}

fn debug_digest(value: &impl std::fmt::Debug) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hasher::write(&mut hasher, format!("{value:?}").as_bytes());
    std::hash::Hasher::finish(&hasher)
}

/// `(median, p10, p90)` in microseconds per tx.
fn percentiles(mut samples: Vec<Duration>, txs: u64) -> (f64, f64, f64) {
    samples.sort();
    let at = |p: usize| samples[(samples.len() - 1) * p / 100].as_secs_f64() * 1e6 / txs as f64;
    (at(50), at(10), at(90))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let txs_per_block = env_or("SOV_BENCH_TXNS_PER_BLOCK", 10_000);
    let num_senders = env_or("SOV_BENCH_SENDERS", 1_000);
    let samples = env_or("SOV_BENCH_SAMPLES", 15) as usize;
    let with_metrics = std::env::var("SOV_BENCH_METRICS").is_ok_and(|v| v == "1");
    assert_eq!(
        txs_per_block % num_senders,
        0,
        "SOV_BENCH_TXNS_PER_BLOCK must be a multiple of SOV_BENCH_SENDERS"
    );

    let timer = Arc::new(SpanTimer::default());
    tracing::subscriber::set_global_default(timer.clone())?;

    let prepare = Instant::now();
    let (mut genesis, roles) = setup::<S, MockZkvm>(num_senders, Default::default());
    // The ZK test genesis funds a randomly generated attester; a fixed address makes the state
    // root reproducible across runs.
    let fixed_attester: <S as sov_modules_api::Spec>::Address =
        CredentialId::from_bytes([0xa7; 32]).into();
    let random_attester = std::mem::replace(
        &mut genesis.attester_incentives.initial_attesters[0].0,
        fixed_attester,
    );
    for (address, _) in &mut genesis
        .bank
        .gas_token_config
        .as_mut()
        .expect("The test genesis has a gas token")
        .address_and_balances
    {
        if *address == random_attester {
            *address = fixed_attester;
        }
    }
    let mut runner = TestRunner::<RT, S, Sm>::new_with_genesis(
        genesis.into_genesis_params(),
        Default::default(),
    );
    runner.config.freeze_time = Some(sov_rollup_interface::da::Time::from_secs(1_700_000_000));
    // Round-major, so each sender's txs are in nonce order within the batch.
    let txs = (0..txs_per_block / num_senders)
        .flat_map(|round| {
            roles.senders.iter().enumerate().map(move |(i, sender)| {
                build_send_tx::<S>(
                    sender,
                    config_gas_token_id(),
                    round * num_senders + i as u64,
                )
            })
        })
        .collect::<Vec<_>>();
    let block = TestRunner::<RT, S, Sm>::soft_confirmation_batches_to_blobs(
        vec![roles.preferred_sequencer.build_preferred_batch::<RT>(txs)],
        runner.nonces_mut(),
    );
    let prepare = prepare.elapsed();

    let mut execute = || {
        let start = Instant::now();
        let (output, _, _) = runner.simulate(block.clone());
        let slot = start.elapsed();
        let user_space = timer.take();
        let successful = output
            .batch_receipts
            .iter()
            .flat_map(|batch| &batch.tx_receipts)
            .filter(|tx| tx.receipt.is_successful())
            .count() as u64;
        assert_eq!(successful, txs_per_block, "Every transfer should succeed");
        (
            user_space,
            slot,
            debug_digest(&output.batch_receipts),
            debug_digest(&output.state_root),
        )
    };

    // Untimed: warms the signature cache, the NOMT page cache and the allocator.
    let (_, _, digest, state_root) = execute();

    let metrics_shutdown = SecondaryShutdownController::new();
    let metrics = if with_metrics {
        Some(start_metrics_collection(&metrics_shutdown).await?)
    } else {
        None
    };

    let mut user_space_times = Vec::with_capacity(samples);
    let mut slot_times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let (user_space, slot, sample_digest, sample_root) = execute();
        assert_eq!(
            sample_digest, digest,
            "Every sample should produce the same receipts"
        );
        assert_eq!(
            sample_root, state_root,
            "Every sample should produce the same state root"
        );
        user_space_times.push(user_space);
        slot_times.push(slot);
    }

    let (median, p10, p90) = percentiles(user_space_times, txs_per_block);
    let (slot_median, slot_p10, slot_p90) = percentiles(slot_times, txs_per_block);
    println!(
        "stf_loop: us_per_tx={median:.2} p10={p10:.2} p90={p90:.2} slot_us_per_tx={slot_median:.2} \
         slot_p10={slot_p10:.2} slot_p90={slot_p90:.2} samples={samples} txs_per_block={txs_per_block} \
         senders={num_senders} receipts_digest={digest:016x} state_root_digest={state_root:016x} prepare_s={:.1}",
        prepare.as_secs_f64()
    );

    if let Some((publisher, collector)) = metrics {
        metrics_shutdown.shutdown();
        publisher.await?;
        let metrics = PhaseMetrics::from_lines(&collector.await?);
        const SLOT: &str = "sov_rollup_slot_execution_time_us";
        const TX: &str = "sov_rollup_transaction_execution_us";
        const AUTH: &str = "sov_rollup_auth_and_process_metrics";
        let per_tx = |sum: f64| sum / (samples as u64 * txs_per_block) as f64;
        println!(
            "stf_loop_metrics: blobs_us_per_tx={:.2} finalization_us_per_tx={:.2} tx_exec_avg_us={:.2} \
             auth_avg_us={:.2} auth_and_process_avg_us={:.2} tx_metrics={} metric_lines={}",
            per_tx(metrics.sum(SLOT, "Node", "blobs_processing")),
            per_tx(metrics.sum(SLOT, "Node", "finalization")),
            metrics.avg(TX, "Node", "value"),
            metrics.avg(AUTH, "Node", "auth_time_us"),
            metrics.avg(AUTH, "Node", "total_time_us"),
            metrics.count(TX, "Node"),
            metrics.lines,
        );
    }
    Ok(())
}
