//! Full-cycle throughput: the preferred sequencer and the node run in one process, as in
//! production.
//!
//! Pre-signed bank transfers are submitted straight to the sequencer's `accept_tx` from
//! concurrent tasks (no HTTP), the sequencer posts batches to a periodic mock DA, and the node
//! re-executes them, reusing the signature verifications the sequencer cached on ingestion. The
//! bench finishes when the node has processed every transaction.
//!
//! Compare with `rollup_coarse_measure`, which runs the STF alone with a cold signature cache
//! (the replica / resync case).
//!
//! Phase timings come from the rollup's own metrics, sent to an in-process TCP listener instead
//! of Telegraf and split by execution context (`Node`, `Sequencer`, `SequencerWarmUp`).
//!
//! Output is one `full_cycle:` line. `e2e_tps` spans first submission to the node processing the
//! last tx; `ingest_tps` spans submission only; `*_us_per_tx` divide a phase's total time by all
//! txs; `*_avg_us` average a per-tx metric. The bench fails if any tx was unsuccessful or any
//! metric was dropped.
//!
//! Knobs: `SOV_BENCH_TXS` (default 50_000), `SOV_BENCH_SENDERS` (1_000),
//! `SOV_BENCH_SUBMITTERS` (64), `SOV_BENCH_METRICS_FILE` (optional path to dump the raw
//! metric lines). The metrics tracker is process-global, so run one rollup per
//! process.

#![allow(clippy::float_arithmetic)]

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use demo_stf::runtime::Runtime;
use sov_benchmarks::node::build_send_tx;
use sov_benchmarks::{setup, NomtBenchSpec};
use sov_metrics::{init_metrics_tracker, MonitoringConfig, TelegrafSocketConfig};
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::{FullyBakedTx, NodeEndpoints, Spec};
use sov_rollup_interface::TxHash;
use sov_sequencer::{Sequencer, SequencerKindConfig, TxStatus};
use sov_shutdown::{BackgroundHandle, PrimaryShutdownController, SecondaryShutdownController};
use sov_stf_runner::RollupConfig;
use sov_test_utils::runtime::sov_bank::config_gas_token_id;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
use sov_test_utils::{AdditionalSequencerApis, AsUser, MockZkvm, RtAgnosticBlueprintWithApis};
use tokio::io::AsyncBufReadExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

type S = NomtBenchSpec;
type RT = Runtime<S>;
type Blueprint = RtAgnosticBlueprintWithApis<S, RT, CaptureSequencer>;
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// `Ok(true)` if the sequencer executed the tx successfully; `Err((http_status, message))` if it
/// refused it.
type AcceptResult = Result<bool, (u16, String)>;

/// Matches `configs/mock_rollup_config.toml`.
const BLOCK_TIME_MS: u64 = 1000;
/// Upper bound for the node to process everything; only reached if the rollup stalls.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);
/// Back-off before resubmitting a tx the sequencer refused because it is busy.
const RETRY_DELAY: Duration = Duration::from_millis(10);
/// Pending-metrics queue. The default (1000) drops per-tx metrics at these rates.
const METRICS_QUEUE: u32 = 10_000_000;

/// Type-erased handle to the in-process preferred sequencer.
struct SequencerHandle {
    accept: Box<dyn Fn(FullyBakedTx) -> BoxFuture<AcceptResult> + Send + Sync>,
    /// Subscribes immediately; the future resolves once the node has processed the tx.
    wait_processed: Box<dyn Fn(TxHash) -> BoxFuture<()> + Send + Sync>,
}

/// Taken by `main`, so the sequencer clone is dropped before shutdown; a live clone keeps the
/// sequencer's background tasks, and with them the rollup, from stopping.
static SEQUENCER: Mutex<Option<SequencerHandle>> = Mutex::new(None);

/// Captures the sequencer the rollup creates. `TestRollup` doesn't expose it, and this hook is
/// the one place the blueprint hands it out.
#[derive(Default)]
struct CaptureSequencer;

impl AdditionalSequencerApis<S, RT> for CaptureSequencer {
    fn create<Seq>(
        sequencer: Seq,
        _rollup_config: &RollupConfig<<S as Spec>::Address, StorableMockDaService>,
        _primary_shutdown: PrimaryShutdownController,
        _sequencer_da_address: MockAddress,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = S, Rt = RT, Da = StorableMockDaService>,
    {
        let seq = sequencer.clone();
        let accept = Box::new(move |tx| {
            let seq = seq.clone();
            Box::pin(async move {
                let accepted = seq
                    .accept_tx(tx, IpAddr::V4(Ipv4Addr::LOCALHOST))
                    .await
                    .map_err(|e| (e.status.as_u16(), e.message))?;
                // The REST API serializes the confirmation the same way for its clients.
                let confirmation = serde_json::to_value(&accepted.confirmation)
                    .expect("Confirmation is serializable");
                Ok(confirmation["receipt"]["result"] == "successful")
            }) as BoxFuture<_>
        });
        let wait_processed = Box::new(move |tx_hash: TxHash| {
            // Subscribe now, not on first poll, so a fast `Processed` can't be missed.
            let (dropper, mut statuses) = sequencer.tx_status_manager().subscribe(tx_hash);
            Box::pin(async move {
                loop {
                    match statuses.recv().await {
                        Ok(TxStatus::Processed) => break,
                        Ok(TxStatus::Dropped { reason }) => {
                            panic!("Sequencer dropped tx {tx_hash}: {reason}")
                        }
                        Ok(_) | Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => panic!("Tx status channel closed for {tx_hash}"),
                    }
                }
                // `SubscriptionDropper` must be dropped after the receiver.
                drop(statuses);
                drop(dropper);
            }) as BoxFuture<()>
        });
        let previous = SEQUENCER.lock().unwrap().replace(SequencerHandle {
            accept,
            wait_processed,
        });
        assert!(previous.is_none(), "Only one rollup per process");
        Ok(NodeEndpoints::default())
    }
}

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |v| {
        v.parse()
            .unwrap_or_else(|_| panic!("{name} must be a positive integer"))
    })
}

/// Starts the process-global metrics tracker, publishing InfluxDB lines over TCP to an
/// in-process collector.
///
/// The rollup's own tracker init is then a no-op (the first init wins), which lets the bench size
/// the queue so no metrics are dropped and flush it after the rollup stops.
async fn start_metrics_collection(
    shutdown: &SecondaryShutdownController,
) -> anyhow::Result<(BackgroundHandle<()>, JoinHandle<Vec<String>>)> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let config = MonitoringConfig {
        telegraf_address: TelegrafSocketConfig::tcp(listener.local_addr()?),
        max_pending_metrics: Some(METRICS_QUEUE),
        ..MonitoringConfig::standard()
    };
    let publisher = init_metrics_tracker(&config, shutdown)
        .expect("The metrics tracker is initialized once, before the rollup starts");
    let collector = tokio::spawn(async move {
        let mut lines = Vec::new();
        // The publisher keeps one connection open until it shuts down. An accept or read error
        // only truncates the metrics; the summary reports how many lines arrived.
        if let Ok((stream, _)) = listener.accept().await {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                lines.push(line);
            }
        }
        lines
    });
    Ok((publisher, collector))
}

/// Field sums and line counts per (measurement, execution context), from the collected lines.
#[derive(Default)]
struct PhaseMetrics {
    lines: usize,
    series: HashMap<(String, String), (HashMap<String, f64>, u64)>,
    /// Node txs per second for each DA height that carried txs.
    node_slot_tps: Vec<f64>,
}

/// The publisher sometimes writes a metric right after the previous one's timestamp, without
/// a newline in between; split there (every measurement name starts with `sov_`).
fn split_glued(line: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = line
        .match_indices("sov_")
        .map(|(i, _)| i)
        .filter(|&i| i > 0 && line.as_bytes()[i - 1].is_ascii_digit())
        .collect();
    starts.insert(0, 0);
    starts.push(line.len());
    starts.windows(2).map(|w| &line[w[0]..w[1]]).collect()
}

impl PhaseMetrics {
    fn from_lines(lines: &[String]) -> Self {
        let mut metrics = Self::default();
        for line in lines.iter().flat_map(|line| split_glued(line)) {
            metrics.lines += 1;
            // `measurement,tag=v,... field=v,... timestamp`
            let mut parts = line.splitn(3, ' ');
            let (Some(head), Some(fields)) = (parts.next(), parts.next()) else {
                continue;
            };
            let mut head = head.split(',');
            let measurement = head.next().unwrap_or_default().to_string();
            let context = head
                .find_map(|tag| tag.strip_prefix("context="))
                .unwrap_or_default()
                .to_string();
            let (sums, count) = metrics.series.entry((measurement, context)).or_default();
            *count += 1;
            for (field, value) in fields.split(',').filter_map(|kv| kv.split_once('=')) {
                // Skips string fields, which none of the summarized metrics use.
                if let Ok(value) = value.trim_end_matches('i').parse::<f64>() {
                    *sums.entry(field.to_string()).or_default() += value;
                }
            }
        }
        metrics.node_slot_tps = Self::node_slot_tps(lines);
        metrics
    }

    /// Joins per-height tx counts with per-height processing time.
    fn node_slot_tps(lines: &[String]) -> Vec<f64> {
        let field = |line: &str, name: &str| -> Option<f64> {
            line.split([' ', ','])
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
        };
        let mut txs_at_height = HashMap::new();
        let mut process_slot_us_at_height = HashMap::new();
        for line in lines.iter().flat_map(|line| split_glued(line)) {
            let Some(height) = field(line, "da_height") else {
                continue;
            };
            if line.starts_with("sov_rollup_runner_counts ") {
                txs_at_height.extend(field(line, "transactions_c").map(|txs| (height as u64, txs)));
            } else if line.starts_with("sov_rollup_runner_times_us ") {
                process_slot_us_at_height
                    .extend(field(line, "process_slot").map(|us| (height as u64, us)));
            }
        }
        let mut tps: Vec<f64> = txs_at_height
            .into_iter()
            .filter(|(_, txs)| *txs > 0.0)
            .filter_map(|(height, txs)| Some(txs / (process_slot_us_at_height.get(&height)? / 1e6)))
            .collect();
        tps.sort_by(f64::total_cmp);
        tps
    }

    fn sum(&self, measurement: &str, context: &str, field: &str) -> f64 {
        self.series
            .get(&(measurement.to_string(), context.to_string()))
            .and_then(|(sums, _)| sums.get(field).copied())
            .unwrap_or(f64::NAN)
    }

    fn count(&self, measurement: &str, context: &str) -> u64 {
        self.series
            .get(&(measurement.to_string(), context.to_string()))
            .map_or(0, |(_, count)| *count)
    }

    fn avg(&self, measurement: &str, context: &str, field: &str) -> f64 {
        self.sum(measurement, context, field) / self.count(measurement, context) as f64
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_some() {
        sov_test_utils::initialize_logging();
    }
    let num_senders = env_or("SOV_BENCH_SENDERS", 1_000);
    let txs_per_sender = env_or("SOV_BENCH_TXS", 50_000).div_ceil(num_senders);
    let total_txs = txs_per_sender * num_senders;
    let num_submitters = env_or("SOV_BENCH_SUBMITTERS", 64) as usize;

    let (genesis, roles) = setup::<S, MockZkvm>(num_senders, Default::default());

    // Not timed: sign everything up front. Each sender's txs stay with one submitter, in nonce
    // order, so the sequencer never sees a nonce gap.
    let mut nonces = HashMap::new();
    let mut batches: Vec<Vec<FullyBakedTx>> = vec![Vec::new(); num_submitters];
    for round in 0..txs_per_sender {
        for (i, sender) in roles.senders.iter().enumerate() {
            let tx = build_send_tx::<S>(
                sender,
                config_gas_token_id(),
                round * num_senders + i as u64,
            )
            .to_serialized_authenticated_tx(&mut nonces);
            batches[i % num_submitters].push(tx);
        }
    }
    batches.retain(|txs| !txs.is_empty());

    let metrics_shutdown = SecondaryShutdownController::new();
    let (metrics_publisher, metrics_collector) =
        start_metrics_collection(&metrics_shutdown).await?;
    let sequencer_info = &roles.preferred_sequencer.sequencer_info;
    let rollup = RollupBuilder::<Blueprint>::new(
        GenesisSource::CustomParams(genesis.into_genesis_params()),
        BlockProducingConfig::Periodic {
            block_time_ms: BLOCK_TIME_MS,
        },
        0,
    )
    .set_config(|c| {
        c.sequencer_address = sequencer_info.as_user().address().to_string();
        // Production values from `configs/mock_rollup_config.toml`.
        c.max_batch_size_bytes = 1024 * 1024;
        c.max_concurrent_batch_blobs = 128;
        if let SequencerKindConfig::Preferred(p) = &mut c.sequencer_config {
            p.ideal_lag_behind_finalized_slot = 3;
            p.batch_execution_time_limit_millis = 2000;
        }
    })
    .set_da_config(|da| da.sender_address = sequencer_info.da_address)
    .start()
    .await?;
    rollup.wait_for_sequencer_ready().await?;
    eprintln!("full_cycle: rollup ready, submitting {total_txs} txs");
    let sequencer = Arc::new(
        SEQUENCER
            .lock()
            .unwrap()
            .take()
            .expect("The rollup creates its sequencer on start"),
    );

    // The last tx of each submitter is processed after all of its earlier ones.
    let completions: Vec<_> = batches
        .iter()
        .map(|txs| {
            let last = txs.last().expect("Empty batches were removed");
            let hash = <<RT as sov_modules_api::Runtime<S>>::Auth as TransactionAuthenticator<S>>::compute_tx_hash(last)
                .expect("Pre-signed tx is well-formed");
            (sequencer.wait_processed)(hash)
        })
        .collect();

    let start = Instant::now();
    let submitters: Vec<_> = batches
        .into_iter()
        .map(|txs| {
            let sequencer = sequencer.clone();
            tokio::spawn(async move {
                let mut not_successful = 0u64;
                let mut retries = HashMap::<String, u64>::new();
                for tx in txs {
                    loop {
                        match (sequencer.accept)(tx.clone()).await {
                            Ok(successful) => {
                                not_successful += u64::from(!successful);
                                break;
                            }
                            // Back-pressure, e.g. the sequencer's block executor queue is full.
                            Err((status @ (503 | 429), message)) => {
                                *retries.entry(format!("{status} {message}")).or_default() += 1;
                                tokio::time::sleep(RETRY_DELAY).await;
                            }
                            Err((status, message)) => {
                                panic!("accept_tx failed with {status}: {message}")
                            }
                        }
                    }
                }
                (not_successful, retries)
            })
        })
        .collect();
    let mut not_successful = 0;
    let mut retry_reasons = HashMap::<String, u64>::new();
    for submitter in submitters {
        let (n, reasons) = submitter.await?;
        not_successful += n;
        for (reason, count) in reasons {
            *retry_reasons.entry(reason).or_default() += count;
        }
    }
    let retries: u64 = retry_reasons.values().sum();
    let ingest_time = start.elapsed();
    eprintln!("full_cycle: sequencer accepted all txs in {ingest_time:?}, waiting for the node");
    for (reason, count) in &retry_reasons {
        eprintln!("full_cycle: {count} retries after: {reason}");
    }
    tokio::time::timeout(COMPLETION_TIMEOUT, async {
        for completion in completions {
            completion.await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Node didn't process all txs within {COMPLETION_TIMEOUT:?}"))?;
    let e2e_time = start.elapsed();
    drop(sequencer);

    rollup.shutdown().await?;
    metrics_shutdown.shutdown();
    metrics_publisher.await?;
    let metric_lines = metrics_collector.await?;
    if let Some(path) = std::env::var_os("SOV_BENCH_METRICS_FILE") {
        std::fs::write(path, metric_lines.join("\n"))?;
    }
    let metrics = PhaseMetrics::from_lines(&metric_lines);

    let node_slot_tps_median = metrics
        .node_slot_tps
        .get(metrics.node_slot_tps.len() / 2)
        .copied()
        .unwrap_or(f64::NAN);
    const SLOT: &str = "sov_rollup_slot_execution_time_us";
    const TX: &str = "sov_rollup_transaction_execution_us";
    const AUTH: &str = "sov_rollup_auth_and_process_metrics";
    let per_tx = |sum: f64| sum / total_txs as f64;
    // Reported every 10 s; absent in a run shorter than that.
    let dropped_metrics = match metrics.count("sov_rollup_dropped_metrics", "") {
        0 => 0.0,
        _ => metrics.sum("sov_rollup_dropped_metrics", "", "dropped_metrics"),
    };
    println!(
        "full_cycle: txs={total_txs} e2e_tps={:.1} ingest_tps={:.1} node_slot_tps_median={node_slot_tps_median:.1} \
         node_blobs_us_per_tx={:.1} node_finalization_us_per_tx={:.1} node_stf_changes_us_per_tx={:.1} \
         node_tx_exec_avg_us={:.1} node_auth_avg_us={:.1} node_auth_and_process_avg_us={:.1} \
         seq_tx_exec_avg_us={:.1} seq_auth_avg_us={:.1} seq_auth_and_process_avg_us={:.1} warmup_tx_exec_avg_us={:.1} \
         node_tx_metrics={} seq_tx_metrics={} not_successful={not_successful} retries={retries} metric_lines={} dropped_metrics={dropped_metrics}",
        total_txs as f64 / e2e_time.as_secs_f64(),
        total_txs as f64 / ingest_time.as_secs_f64(),
        per_tx(metrics.sum(SLOT, "Node", "blobs_processing")),
        per_tx(metrics.sum(SLOT, "Node", "finalization")),
        per_tx(metrics.sum("sov_runner_process_stf_changes", "", "total_time_us")),
        metrics.avg(TX, "Node", "value"),
        metrics.avg(AUTH, "Node", "auth_time_us"),
        metrics.avg(AUTH, "Node", "total_time_us"),
        metrics.avg(TX, "Sequencer", "value"),
        metrics.avg(AUTH, "Sequencer", "auth_time_us"),
        metrics.avg(AUTH, "Sequencer", "total_time_us"),
        metrics.avg(TX, "SequencerWarmUp", "value"),
        metrics.count(TX, "Node"),
        metrics.count(TX, "Sequencer"),
        metrics.lines,
    );

    assert_eq!(
        not_successful, 0,
        "Every pre-signed transfer should execute successfully"
    );
    assert_eq!(
        dropped_metrics, 0.0,
        "Dropped metrics make the phase timings incomplete"
    );
    Ok(())
}
