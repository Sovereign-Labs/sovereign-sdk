//! This file generate harness files and stores them in the harnesses `generated` folder.

use std::env;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use clap::Parser;
use sov_benchmarks::bench_generator::benches::all_benches;
use sov_benchmarks::bench_generator::cli::BenchCLI;
use sov_test_utils::initialize_logging;
use tracing::{info, info_span};

#[tokio::main]
async fn main() {
    let params = BenchCLI::parse();

    // If the env var is not set, then we set it to the default value.
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "warn,error,bench_generator=info");
    }

    initialize_logging();

    info_span!("bench_generator");

    let args = params.parse_size();

    let benchmark_sets = all_benches(args, params.slots, params.seed);

    let mut joinset = tokio::task::JoinSet::new();

    for (set_name, benchmarks) in benchmark_sets {
        let mut generation_base_path = PathBuf::from(params.path.clone());

        generation_base_path.push(set_name);

        std::fs::create_dir_all(&generation_base_path).unwrap_or_else(|_| {
            panic!(
                "Impossible to create the generation base path {generation_base_path:?}, please ensure the path is valid"
            )
        });

        for benchmark in benchmarks {
            let generation_base_path_cloned = generation_base_path.clone();
            joinset.spawn(async move {
                let mut bench_with_extension = benchmark.name.clone();
                let bench_stamp = humantime::Timestamp::from(SystemTime::now());
                bench_with_extension.push_str(&format!("_{bench_stamp}.bin"));

                info!(bench = benchmark.name, "Generating benchmark...");

                let path = generation_base_path_cloned.join(bench_with_extension);
                let path_str = path
                    .to_str()
                    .expect("Path should be well encoded")
                    .to_string();

                let file = File::create(path).unwrap_or_else(|_| {
                    panic!(
                    "Impossible to generate a file at path {path_str:?}, please ensure the path is valid",
                )
                });

                let begin_stamp = Instant::now();
                benchmark
                    .generate_and_write_benchmark_messages(&mut BufWriter::new(file))
                    .unwrap_or_else(|_| {
                        panic!(
                            "Impossible to generate benchmark messages for the benchmark {:?}",
                            benchmark.name
                        )
                    });
                let end_stamp = Instant::now();

                let exec_duration = end_stamp.duration_since(begin_stamp);

                info!(
                    bench = benchmark.name,
                    generation_duration = format!(
                        "{}.{}s",
                        exec_duration.as_secs(),
                        exec_duration.subsec_millis()
                    ),
                    "Generation complete...",
                );
            });
        }
    }

    joinset.join_all().await;
}
