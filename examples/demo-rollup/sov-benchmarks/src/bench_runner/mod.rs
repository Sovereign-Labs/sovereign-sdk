//! A file that contains utilities to run benchmarks and gather metrics.

use std::fs::File;
use std::io::BufReader;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context};
use cli::BenchRunnerCLI;
use demo_stf::runtime::{GenesisConfig, Runtime, RuntimeCall};
use helpers::{BatchReceiver, BatchSender};
use humantime::Timestamp;
use sov_metrics::{timestamp, TelegrafSocketConfig};
use sov_mock_da::BlockProducingConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::RtAgnosticBlueprint;
use sov_transaction_generator::generators::basic::{BasicChangeLogEntry, BasicClientConfig};
use sov_transaction_generator::{assert_logs_against_state, GeneratedMessage};
use tokio::sync::mpsc;
use tracing::{info, trace};

use crate::bench_generator::BenchmarkData;
use crate::bench_runner::cli::MetricsCLI;
use crate::{mock_da_risc0_host_args, BenchRisc0Spec, DEFAULT_FINALIZATION_BLOCKS};

pub type S = BenchRisc0Spec;
pub type RT = Runtime<S>;
pub type BenchBlueprint = RtAgnosticBlueprint<S, RT>;
pub type BenchRollup = TestRollup<BenchBlueprint>;
pub type BenchRollupBuilder = RollupBuilder<BenchBlueprint>;
pub type BenchLogs = BasicChangeLogEntry<S>;
pub type BenchMessage = GeneratedMessage<S, RuntimeCall<S>, BenchLogs>;

pub mod cli;
pub mod helpers;
pub mod metrics;

pub const DEFAULT_BENCH_FILES: &str = "./src/bench_files";
pub const DEFAULT_METRICS_OUTPUT: &str = "./src/metrics";
pub const DEFAULT_TELEGRAF_ADDRESS: TelegrafSocketConfig =
    TelegrafSocketConfig::tcp(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
        std::net::Ipv4Addr::LOCALHOST,
        8094,
    )));
pub const DEFAULT_INFLUX_DB_ADDRESS: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8086));
pub const DEFAULT_NUM_THREADS: u8 = 10;

/// Number of slots the DA layer has to produce before the sequencer can start producing batches.
const BOOTSTRAP_SLOTS_NUM: usize = 2;

const AGG_PROOF_JUMP: u64 = 1;

pub struct ParsedMetricsParameters {
    pub(crate) influx_address: SocketAddr,
    pub(crate) output_file: String,
    pub(crate) query_filter: String,
    pub(crate) influx_auth_token: String,
    pub(crate) influx_org_id: String,
    pub(crate) encoded: bool,
}

/// Parses the next slot from a bench file.
pub fn parse_next_data(reader: &mut BufReader<File>) -> anyhow::Result<BenchmarkData<S>> {
    Ok(bincode::deserialize_from(reader)?)
}

/// Setups the rollup for the benchmarks.
/// We give the maximum possible gas balance to the prover and sequencer to ensure that they can pay for the transactions.
pub async fn setup_rollup(
    genesis_config: GenesisConfig<S>,
    telegraf_address: TelegrafSocketConfig,
) -> anyhow::Result<BenchRollup> {
    let sequencer_da_address = genesis_config
        .sequencer_registry
        .sequencer_config
        .seq_da_address;
    let prover_address = genesis_config
        .prover_incentives
        .initial_provers
        .first()
        .unwrap()
        .0;

    let rollup_builder = BenchRollupBuilder::new(
        GenesisSource::CustomParams(genesis_config.into_genesis_params()),
        BlockProducingConfig::Manual,
        DEFAULT_FINALIZATION_BLOCKS,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|config| {
        config.max_concurrent_blobs = 1024;
        config.prover_address = prover_address.to_string();
        config.automatic_batch_production = true;
        config.telegraf_address = telegraf_address;
        config.aggregated_proof_block_jump = AGG_PROOF_JUMP as usize;

        // This value should be greater than the number of slots we want to run as part of the benchmark.
        config.max_channel_size = 2000;
        config.max_infos_in_db = 2000;
    })
    .set_da_config(|da_config| {
        da_config.sender_address = sequencer_da_address;
    });

    rollup_builder.start().await
}

/// Runs a bench file and gathers metrics.
async fn runner(
    bench_name: String,
    bench_file: File,
    maybe_logs: Option<u8>,
    telegraf_address: TelegrafSocketConfig,
) -> anyhow::Result<()> {
    // Starts by setting up the rollup for the benchmarks.
    let mut reader = BufReader::new(bench_file);

    let Ok(BenchmarkData::Genesis(genesis_config)) = parse_next_data(&mut reader) else {
        bail!("{bench_name}: The bench file should start with an initialization slot. The bench file is invalid");
    };
    let rollup = setup_rollup(genesis_config, telegraf_address).await?;

    info!(bench = bench_name, "Bootstrapping the rollup");

    // Bootstrap the rollup by producing blocks.
    rollup
        .da_service
        .produce_n_blocks_now(BOOTSTRAP_SLOTS_NUM)
        .await?;

    // Sleep to be sure the sequencer state is up to date.
    sleep(Duration::from_secs(1));

    info!(bench = bench_name, "Starting batch submission");

    let (batch_sender, batch_receiver) = mpsc::channel(500);
    let mut batch_sender =
        BatchSender::new(bench_name.clone(), rollup.client.clone(), batch_sender).await;
    let batch_receiver = BatchReceiver::new(
        bench_name.clone(),
        rollup.client.clone(),
        batch_receiver,
        &rollup.da_service,
    )
    .await;

    let receiver_handle = batch_receiver.start_receiver();

    let Ok(BenchmarkData::Initialization(init_slot)) = parse_next_data(&mut reader) else {
        bail!("{bench_name}: The bench file should start with an initialization slot. The bench file is invalid");
    };

    let mut log_accumulator = maybe_logs.map(|_| Vec::<BenchLogs>::new());

    let mut logs = batch_sender.send_txs_to_sequencer(init_slot).await?;
    if let Some(acc) = log_accumulator.as_mut() {
        acc.append(&mut logs)
    }

    rollup.da_service.produce_block_now().await?;

    // We need to wait to be sure the sequencer sends the first transactions to the DA layer.
    tokio::time::sleep(Duration::from_secs(1)).await;

    while let Ok(bench) = parse_next_data(&mut reader) {
        let slot_start = timestamp();
        let (slot_number, slot) = match bench {
            BenchmarkData::Execution {
                batches,
                slot_number,
            } => {
                trace!(bench = bench_name, slot = slot_number, "Executing slot...");
                (slot_number, batches)
            }
            _ => {
                panic!("{bench_name}: Expected an execution slot.")
            }
        };

        // FIXME: here we flatten the batches because the preferred sequencer can only send one batch per slot.
        // Ideally we should have a batch sender that can send multiple batches per slot.
        let batches = slot.into_iter().flatten().collect::<Vec<_>>();

        let num_txs = batches.len();

        let mut logs = batch_sender
            .send_txs_to_sequencer(batches)
            .await
            .with_context(|| format!("{bench_name}: Failed to produce and publish batch"))?;
        if let Some(acc) = log_accumulator.as_mut() {
            acc.append(&mut logs)
        }
        let slot_end = timestamp();

        rollup.da_service.produce_block_now().await?;

        // We need to wait to be sure the sequencer sends the last transactions to the DA layer to be included in the next block.
        tokio::time::sleep(Duration::from_secs(2)).await;

        let exec_duration = Duration::from_nanos((slot_end - slot_start) as u64);
        let throughput = num_txs as f64 / exec_duration.as_secs_f64();

        info!(
            bench = bench_name,
            slot = slot_number,
            transactions = num_txs,
            duration_ms = exec_duration.as_millis(),
            throughtput = throughput.round(),
            "Slot execution completed",
        );
    }

    // We wait for all the results to be in.
    info!(
        bench = bench_name,
        thread = "runner",
        "Waiting for submission results for the last slots..."
    );

    // We drop the sender to ensure that the sender thread is closed which triggers completion of the receiver handle.
    drop(batch_sender);

    receiver_handle.await??;

    // Assert logs (if necessary) and shut down the rollup afterwards.
    // Note that to assert the logs, the rollup still must be running (for the REST-api to be available).
    if let Some((assert_logs, log_accumulator)) = maybe_logs.zip(log_accumulator) {
        info!(bench = bench_name, thread = "shutdown", "Asserting logs...");
        assert_logs_against_state(
            log_accumulator,
            Arc::new(BasicClientConfig {
                url: rollup.api_client().baseurl().clone(),
                rollup_height: None,
            }),
            assert_logs,
        )
        .await
        .expect("Failed to assert logs");
    }

    info!(
        bench = bench_name,
        thread = "shutdown",
        "Shutting down rollup..."
    );

    rollup
        .shutdown_sender
        .send(())
        .expect("Failed to send shutdown signal");
    let _x = rollup
        .rollup_task
        .await
        .expect("Failed to join rollup task");

    Ok(())
}

pub async fn run_bench_file(input: BenchRunnerCLI) {
    // Collect and store metrics to file in a separate thread.
    let (metrics_shutdown_sender, metrics_shutdown_receiver) =
        tokio::sync::watch::channel::<()>(());

    let telegraf_address = if let Some(MetricsCLI::Metrics { telegraf, .. }) = input.metrics {
        telegraf
    } else {
        DEFAULT_TELEGRAF_ADDRESS
    };

    let file_path = PathBuf::from(input.path.clone());
    let bench_file = File::open(file_path.clone())
        .unwrap_or_else(|_| panic!("Failed to open bench file at path {}. Make sure you provided an appropriate file name!", file_path.display()));

    let bench_name = file_path.clone();
    let bench_name = bench_name.file_stem().unwrap();

    let bench_name_str = bench_name.to_str().unwrap();

    info!(path = bench_name_str, "Running bench file");

    // Parse the metrics CLI parameters.
    if let Some(MetricsCLI::Metrics {
        influx,
        influx_auth_token,
        influx_org_id,
        output,
        encoded,
        parameters,
        ..
    }) = input.metrics
    {
        let bench_name = bench_name_str.to_string();

        // Spawning the metrics collection task in a separate thread.
        tokio::spawn(async move {
            {
                let metrics_params = {
                    let output_dir = PathBuf::from(output);

                    // If the metrics are encoded, then we need to add the .gzip extension.
                    let mut path_with_stamp = PathBuf::from(format!(
                        "{}_{}",
                        file_path.file_stem().unwrap().to_str().unwrap(),
                        Timestamp::from(SystemTime::now())
                    ));

                    if encoded {
                        path_with_stamp.set_extension("csv.gz");
                    } else {
                        path_with_stamp.set_extension("csv");
                    }

                    ParsedMetricsParameters {
                        output_file: output_dir.join(path_with_stamp).display().to_string(),
                        influx_address: influx,
                        influx_auth_token,
                        influx_org_id,
                        encoded,
                        query_filter: parameters.map(|p| p.format()).unwrap_or_default(),
                    }
                };

                let start_timestamp = timestamp();
                metrics::start_metrics_thread(
                    bench_name,
                    start_timestamp,
                    metrics_params,
                    metrics_shutdown_receiver,
                )
                .await
                .expect("Failed to collect metrics");
            }
        });
    }

    // Run the bench file.
    runner(
        String::from(bench_name_str),
        bench_file,
        input.logs,
        // We always start the metrics sender for any rollup. If we don't expect metrics, we just pass the default address.
        telegraf_address,
    )
    .await
    .unwrap_or_else(|e| panic!("{bench_name_str}: Impossible to run bench file. Err {e})"));

    // Shutdown the metrics collection task
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2737
    metrics_shutdown_sender.send(()).unwrap();
    tokio::time::sleep(Duration::from_secs(15)).await;
}
