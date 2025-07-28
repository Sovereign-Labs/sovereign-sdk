#![allow(clippy::float_arithmetic)]

use std::default::Default;
use std::env;
use std::time::{Duration, Instant};

use humantime::format_duration;
use prettytable::{row, Table};
use sov_benchmarks::node::{generate_transfers, prefill_state};
use sov_benchmarks::setup_with_runner;
use sov_modules_api::prelude::anyhow;

// Minimum TPS below which it is considered an issue
const MIN_TPS: f64 = 1000.0;
// Number to check that rollup actually executed some transactions
const MAX_TPS: f64 = 30_000.0;

fn print_times(
    total: Duration,
    apply_block_time: Duration,
    blocks: u64,
    num_txns: u64,
    num_success_txns: u64,
) {
    let mut table = Table::new();

    let total_txns = blocks * num_txns;
    table.add_row(row!["Blocks", format!("{:?}", blocks)]);
    table.add_row(row!["Transactions per block", format!("{:?}", num_txns)]);
    table.add_row(row![
        "Processed transactions (success/total)",
        format!("{:?}/{:?}", num_success_txns, total_txns)
    ]);
    table.add_row(row!["Total", format_duration(total)]);
    table.add_row(row!["Apply block", format_duration(apply_block_time)]);
    let tps = (total_txns as f64) / total.as_secs_f64();
    table.add_row(row!["Transactions per sec (TPS)", format!("{:.1}", tps)]);

    // Print the table to stdout
    table.printstd();

    assert!(
        tps > MIN_TPS,
        "TPS {tps} dropped below {MIN_TPS}, investigation is needed"
    );
    assert!(
        tps < MAX_TPS,
        "TPS {tps} reached unrealistic number {MAX_TPS}, investigation is needed"
    );
}

#[derive(Debug)]
struct BenchParams {
    blocks: u64,
    transactions_per_block: u64,
    timer_output: bool,
}

impl BenchParams {
    fn new() -> Self {
        let mut blocks: u64 = 100;
        let mut transactions_per_block = 1000;
        let mut timer_output = true;

        if let Ok(val) = env::var("SOV_BENCH_TXNS_PER_BLOCK") {
            transactions_per_block = val
                .parse()
                .expect("SOV_BENCH_TXNS_PER_BLOCK var should be a +ve number");
        }
        if let Ok(val) = env::var("SOV_BENCH_BLOCKS") {
            blocks = val
                .parse::<u64>()
                .expect("SOV_BENCH_BLOCKS var should be a positive integer");
        }
        if let Ok(val) = env::var("SOV_BENCH_TIMER_OUTPUT") {
            match val.as_str() {
                "true" | "1" | "yes" => {
                    timer_output = true;
                }
                "false" | "0" | "no" => (),
                val => {
                    panic!(
                        "Unknown value '{val}' for SOV_BENCH_TIMER_OUTPUT. expected true/false/0/1/yes/no"
                    );
                }
            }
        }

        Self {
            blocks,
            transactions_per_block,
            timer_output,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let params = BenchParams::new();
    let mut num_success_txns: u64 = 0;

    let (mut runner, roles) = setup_with_runner(params.transactions_per_block, Default::default());
    let token_id = prefill_state(&roles, &mut runner);
    let blocks = generate_transfers(params.blocks, token_id, &roles, &mut runner);

    let expected_num_txs: u64 = params.blocks * params.transactions_per_block;

    let blocks_num = params.blocks;

    let total = Instant::now();
    let mut apply_block_time = Duration::default();

    for filtered_block in blocks {
        let now = Instant::now();
        let apply_block_result = runner.execute(filtered_block);
        apply_block_time += now.elapsed();

        for receipt in apply_block_result.0.batch_receipts {
            for t in &receipt.tx_receipts {
                if t.receipt.is_successful() {
                    num_success_txns += 1;
                } else {
                    println!("E: {:?}", t.receipt);
                }
            }
        }
    }

    let total = total.elapsed();
    assert_eq!(
        expected_num_txs, num_success_txns,
        "Not enough successful transactions, something is broken"
    );
    if params.timer_output {
        print_times(
            total,
            apply_block_time,
            blocks_num,
            params.transactions_per_block,
            num_success_txns,
        );
    }
    Ok(())
}
