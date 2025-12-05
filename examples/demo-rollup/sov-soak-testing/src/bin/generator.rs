use std::time::Duration;

use clap::Parser;
use sov_bank::Bank;
use sov_modules_api::prelude::tracing;
use sov_modules_api::{EncodeCall, Runtime, Spec};
use sov_soak_testing::{
    spawn_throughput_watcher, CelestiaRollupSpec, DemoCelestiaRT, DemoMockRT, MockDemoRollupSpec,
    SoakTestRunner, TestRT, ValidityProfile, WorkerStats,
};
use sov_synthetic_load::SyntheticLoad;
use sov_test_utils::TestSpec;
use sov_transaction_generator::interface::MessageValidity;
use sov_transaction_generator::Distribution;
use tokio::signal::unix::SignalKind;
use tokio::sync::watch::Receiver;
use tokio::task::JoinSet;

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum SelectedRuntime {
    /// Generated test runtime, running by the sov-soak-testing
    Test,
    /// demo-stf with Celestia DA
    DemoCelestia,
    /// demo-stf with Mock DA
    DemoMock,
}

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "http://localhost:12346")]
    /// The URL of the rollup node to connect to. Defaults to http://localhost:12346.
    api_url: String,

    #[arg(short, long, default_value = "5")]
    /// The number of workers to spawn - this controls the number of concurrent transactions. Defaults to 5.
    num_workers: u32,

    #[arg(short, long, default_value = "test")]
    runtime: SelectedRuntime,

    #[arg(short, long, default_value = "0")]
    /// The salt to use for RNG. Use this value if you're restarting the generator and want to ensure that the generated
    /// transactions don't overlap with the previous run.
    salt: u32,

    #[arg(short, long, default_value = "clean")]
    /// The distribution of valid/invalid transactions to generate.
    validity_profile: ValidityProfile,

    #[arg(short, long, default_value = "mixed")]
    /// The distribution of token transfers vs. synthetic load transactions to generate.
    tx_type: TxType,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum TxType {
    /// Only [`SyntheticLoad`] transactions - includes many heavy txs
    SyntheticLoad,
    /// Only [`Bank`] transactions
    Bank,
    /// Mixed [`SyntheticLoad`] and [`Bank`] transactions
    Mixed,
}

/// Helper to create the Runner for any demo runtime.
/// All the demo rollup runtimes only support Bank and SyntheticLoad.
async fn run_soak_test_with_demo_runtime<R, S>(
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
    tx_type: TxType,
    restart_after: Option<std::time::Duration>,
    stats_tx: tokio::sync::mpsc::UnboundedSender<WorkerStats>,
) -> anyhow::Result<()>
where
    R: Runtime<S> + EncodeCall<Bank<S>> + EncodeCall<SyntheticLoad<S>> + Clone,
    S: Spec,
{
    let mut runner = SoakTestRunner::<R, S>::new();

    runner = match tx_type {
        TxType::Bank => runner.with_bank(),
        TxType::SyntheticLoad => runner.with_synthetic_load(),
        TxType::Mixed => runner.with_bank().with_synthetic_load(),
    };

    runner
        .run(
            client,
            rx,
            worker_id,
            num_workers,
            validity,
            restart_after,
            stats_tx,
        )
        .await
}

async fn worker_task(
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    runtime: SelectedRuntime,
    validity_profile: ValidityProfile,
    tx_type: TxType,
    restart_after: Option<std::time::Duration>,
    stats_tx: tokio::sync::mpsc::UnboundedSender<WorkerStats>,
) -> anyhow::Result<()> {
    let validity = validity_profile.get_validity();

    let result = match runtime {
        SelectedRuntime::Test => {
            run_soak_test_with_demo_runtime::<TestRT, TestSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
                restart_after,
                stats_tx,
            )
            .await
        }
        SelectedRuntime::DemoCelestia => {
            run_soak_test_with_demo_runtime::<DemoCelestiaRT, CelestiaRollupSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
                restart_after,
                stats_tx,
            )
            .await
        }
        SelectedRuntime::DemoMock => {
            run_soak_test_with_demo_runtime::<DemoMockRT, MockDemoRollupSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
                restart_after,
                stats_tx,
            )
            .await
        }
    };

    if let Err(e) = result {
        tracing::error!("Worker task {worker_id} failed: {}", e);
        std::process::exit(1);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    let _guard = sov_modules_rollup_blueprint::logging::initialize_logging();
    let mut worker_set = JoinSet::new();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let reqwest_client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(60))
        .read_timeout(Duration::from_secs(120))
        .build()?;
    let client = sov_api_spec::Client::new_with_client(&args.api_url, reqwest_client);

    let restart_after = args
        .restart_after_seconds
        .map(std::time::Duration::from_secs);

    // Create stats channels for each worker
    let mut stats_receivers = Vec::new();
    for i in 0..args.num_workers {
        let (stats_tx, stats_rx) = tokio::sync::mpsc::unbounded_channel();
        stats_receivers.push(stats_rx);

        worker_set.spawn(worker_task(
            client.clone(),
            rx.clone(),
            (i + args.salt) as u128,
            args.num_workers,
            args.runtime,
            args.validity_profile,
            args.tx_type,
            restart_after,
            stats_tx,
        ));
    }

    // Spawn the throughput watcher task
    let watcher_handle = tokio::spawn(spawn_throughput_watcher(stats_receivers, 5));

    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .expect("Failed to set up SIGTERM handler");
    let mut quit =
        tokio::signal::unix::signal(SignalKind::quit()).expect("Failed to set up SIGQUIT handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
        _ = terminate.recv() => tracing::info!("Received SIGTERM"),
        _ = quit.recv() => tracing::info!("Received SIGQUIT"),
    }

    tx.send(true)?;
    _ = worker_set.join_all();

    // Wait for watcher to finish processing remaining stats
    _ = watcher_handle.await;

    Ok(())
}
