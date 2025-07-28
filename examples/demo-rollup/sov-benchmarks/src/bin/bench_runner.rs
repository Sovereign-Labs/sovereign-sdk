//! Runs a benchmark of the zkvm metrics.

use std::env;
use std::fs::{self};
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use sov_benchmarks::bench_runner::cli::BenchRunnerCLI;
use sov_benchmarks::bench_runner::run_bench_file;
use sov_test_utils::initialize_logging;
use tracing::info_span;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse the path of the bench files.
    let cli = BenchRunnerCLI::parse();

    // We only set the logging level if the env var is not set.
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "warn,error,sov_benchmarks=info");
    }

    initialize_logging();

    info_span!("bench_runner");

    let path = PathBuf::from(&cli.path);

    if !path.exists() {
        panic!(
            "Bench file path {path:?} does not exist. Please make sure you provided a valid path."
        );
    }

    // If the path points to a file, then we simply run the bench file.
    if path.is_file() {
        run_bench_file(cli).await;
        return Ok(());
    }

    // Max concurrent processes
    const MAX_PROCESSES: usize = 4;

    // If the path points to a directory, then we run all the bench files inside the directory.
    let bench_files = fs::read_dir(path).unwrap_or_else(|err| panic!("Failed to read bench directory: {err}. Please make sure you provide a valid directory or file path!"));

    // Spawn a child process for each bench file. Await the processes concurrently
    let mut processes = FuturesUnordered::new();

    for bench_file in bench_files {
        // We remove both the path and its argument if it exist. They are replaced when calling the process.
        // Note that `position` consumes the underlying iterator, hence the need to call [`args_os`] again afterwards.
        let maybe_pos = std::env::args_os()
            .position(|item| item.to_str().unwrap() == "-p" || item.to_str().unwrap() == "--path");

        let args = std::env::args_os();

        let args: Vec<_> = if let Some(pos) = maybe_pos {
            args.enumerate()
                .filter_map(|(i, item)| {
                    if i != pos && i != pos + 1 {
                        Some(item)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            args.collect()
        };

        let file_path = bench_file.unwrap().path();
        let file_path_str = file_path.into_os_string().into_string().unwrap();

        if processes.len() >= MAX_PROCESSES {
            tracing::info!(
                "Max concurrent processes reached. Waiting for the oldest spawned process to finish..."
            );

            let (bench, status) = processes
                .next()
                .await
                .ok_or(anyhow::anyhow!("No processes to wait for."))?;

            tracing::info!(bench, ?status, "Bench file execution completed");
        }

        processes.push(async {
            // The first argument is always the process name.
            let mut handle = tokio::process::Command::new(args[0].clone())
                .args(vec!["-p", &file_path_str.clone()])
                .args(args.into_iter().skip(1))
                .kill_on_drop(true)
                .spawn()
                .with_context(|| "Impossible to create child process")
                .unwrap();

            let res = handle.wait().await.unwrap();
            (file_path_str, res)
        });
    }

    while let Some((bench, status)) = processes.next().await {
        tracing::info!(bench, ?status, "Bench file execution completed");
    }

    tracing::info!("All bench files have been run successfully.");

    Ok(())
}
