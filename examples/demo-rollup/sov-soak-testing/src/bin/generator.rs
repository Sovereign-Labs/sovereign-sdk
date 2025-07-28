use std::time::Duration;

use clap::Parser;
use sov_modules_api::prelude::tracing;
use sov_soak_testing::{
    run_generator_task_for_bank_and_synthetic_load, CelestiaRollupSpec, DemoCelestiaRT, DemoMockRT,
    MockDemoRollupSpec, TestRT, TxType, ValidityProfile,
};
use sov_test_utils::TestSpec;
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

    #[arg(short, long, default_value = "buzzy")]
    /// The distribution of valid/invalid transactions to generate.
    validity_profile: ValidityProfile,

    #[arg(short, long, default_value = "mixed")]
    /// The distribution of token transfers vs. synthetic load transactions to generate.
    tx_type: TxType,
}

async fn worker_task(
    client: sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    runtime: SelectedRuntime,
    validity_profile: ValidityProfile,
    tx_type: TxType,
) -> anyhow::Result<()> {
    let validity = validity_profile.get_validity();

    let result = match runtime {
        SelectedRuntime::Test => {
            run_generator_task_for_bank_and_synthetic_load::<TestRT, TestSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
            )
            .await
        }
        SelectedRuntime::DemoCelestia => {
            run_generator_task_for_bank_and_synthetic_load::<DemoCelestiaRT, CelestiaRollupSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
            )
            .await
        }
        SelectedRuntime::DemoMock => {
            run_generator_task_for_bank_and_synthetic_load::<DemoMockRT, MockDemoRollupSpec>(
                client,
                rx,
                worker_id,
                num_workers,
                validity,
                tx_type,
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

    for i in 0..args.num_workers {
        worker_set.spawn(worker_task(
            client.clone(),
            rx.clone(),
            (i + args.salt) as u128,
            args.num_workers,
            args.runtime,
            args.validity_profile,
            args.tx_type,
        ));
    }

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

    Ok(())
}
