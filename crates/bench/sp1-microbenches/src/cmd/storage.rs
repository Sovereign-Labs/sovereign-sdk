use anyhow::Context;
use clap::Args;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use sov_gas_tools::fit::{fit_linear, LinearFit};
use crate::{load_guest_elf, round_at_least_one, BenchResult};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-storage/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-storage",
);

const DEFAULT_ITERATIONS: u32 = 100;
const DEFAULT_BASELINE_ITERATIONS: u32 = 10;
const DEPTHS: &[u32] = &[1, 2, 4, 8, 16, 32, 64, 128, 256];

/// Worst-case trie depth we price every access at. Honest accesses are shallower (~log2 of state
/// size); forcing depth D requires ~2^D hash grinding, so D=64 is infeasible to exceed while only
/// over-charging typical accesses ~2x. See the storage-gas-calibration memory for the rationale.
const TARGET_DEPTH: f64 = 64.0;

// Mode flags must match guest-storage/src/main.rs.
const MODE_READ: u8 = 0;
const MODE_WRITE: u8 = 1;

#[derive(Args, Debug)]
pub struct StorageArgs {
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
    #[arg(long, default_value_t = DEFAULT_BASELINE_ITERATIONS)]
    pub baseline_iterations: u32,
}

/// Sweeps NOMT proof verification over trie depth and prices each access at `TARGET_DEPTH`.
pub fn run(args: StorageArgs) -> anyhow::Result<()> {
    let StorageArgs {
        iterations,
        baseline_iterations,
    } = args;

    println!("\n========== STEP 1: read (verify_multi_proof) ==========");
    let fit_read = run_sweep("read", MODE_READ, iterations, baseline_iterations)?;
    print_fit(&fit_read);

    println!("\n========== STEP 2: write (verify_multi_proof_update) ==========");
    let fit_write = run_sweep("write", MODE_WRITE, iterations, baseline_iterations)?;
    print_fit(&fit_write);

    let access = fit_read.bias + fit_read.per_byte * TARGET_DEPTH;
    let update = fit_write.bias + fit_write.per_byte * TARGET_DEPTH;

    println!("\n========== SUMMARY ==========");
    println!("Raw fitted values (prover gas), cost = bias + per_sibling * depth:");
    println!(
        "  read:  bias = {:.2}, per_sibling = {:.4}",
        fit_read.bias, fit_read.per_byte
    );
    println!(
        "  write: bias = {:.2}, per_sibling = {:.4}",
        fit_write.bias, fit_write.per_byte
    );
    println!("Cost at TARGET_DEPTH = {TARGET_DEPTH:.0}:");
    println!("  access (verify)        = {access:.2}");
    println!("  update (verify_update) = {update:.2}");

    println!(
        "\nSuggested constants.toml values (full value charged to a single dimension, rounded ≥1):"
    );
    println!(
        "  GAS_TO_CHARGE_PER_STORAGE_ACCESS = [{}, 0]",
        round_at_least_one(access)
    );
    println!(
        "  BIAS_STORAGE_UPDATE              = [{}, 0]",
        round_at_least_one(update)
    );
    println!(
        "\nNot proof-driven (NOMT proof cost is depth-driven, values hashed to 32 bytes; value\n\
         hashing already billed via the hash constants). Leave near-zero unless a value-size\n\
         materialization sweep says otherwise:"
    );
    println!("  GAS_TO_CHARGE_PER_READ                = [1, 0]");
    println!("  GAS_TO_CHARGE_PER_BYTE_READ           = [1, 0]");
    println!("  GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE = [1, 0]");

    Ok(())
}

/// Runs the high/low-iteration depth sweep, differences out per-execution setup, and fits a line.
fn run_sweep(
    name: &str,
    mode: u8,
    iterations: u32,
    baseline_iterations: u32,
) -> anyhow::Result<LinearFit> {
    anyhow::ensure!(
        iterations > baseline_iterations,
        "--iterations must be greater than --baseline-iterations"
    );

    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results_high = Vec::with_capacity(DEPTHS.len());
    let mut results_low = Vec::with_capacity(DEPTHS.len());
    for &depth in DEPTHS {
        for (iter_count, results) in [
            (iterations, &mut results_high),
            (baseline_iterations, &mut results_low),
        ] {
            println!("[run] storage {name} depth={depth} iterations={iter_count}");
            let mut stdin = SP1Stdin::new();
            stdin.write(&mode);
            stdin.write(&depth);
            stdin.write(&iter_count);
            let report = execute_and_collect(&client, elf.clone(), stdin, depth, iter_count)
                .with_context(|| format!("{name} failed at depth={depth} iter={iter_count}"))?;
            results.push(report);
        }
    }

    let (depths_f, per_iter_gas) =
        differential(&results_high, &results_low, iterations, baseline_iterations);
    let fit = fit_linear(&depths_f, &per_iter_gas)?;

    print_raw_table("high-iter", &results_high);
    print_raw_table("low-iter (baseline)", &results_low);
    print_differential_table(&depths_f, &per_iter_gas);

    Ok(fit)
}

fn execute_and_collect(
    client: &impl Prover,
    elf: sp1_sdk::blocking::Elf,
    stdin: SP1Stdin,
    depth: u32,
    iterations: u32,
) -> anyhow::Result<BenchResult> {
    let (_pv, report) = client.execute(elf, stdin).run()?;
    let prover_gas = report
        .gas()
        .context("prover gas not available; ProverClient may have disabled gas calculation")?;
    let total_cycles = report.total_instruction_count();
    let region_cycles = report
        .cycle_tracker
        .get("storage_loop")
        .copied()
        .unwrap_or(0);
    Ok(BenchResult {
        input_size: depth,
        iterations,
        prover_gas,
        total_cycles,
        region_cycles,
    })
}

fn differential(
    results_high: &[BenchResult],
    results_low: &[BenchResult],
    iterations: u32,
    baseline_iterations: u32,
) -> (Vec<f64>, Vec<f64>) {
    let iter_delta = f64::from(iterations - baseline_iterations);
    let depths: Vec<f64> = results_high
        .iter()
        .map(|r| f64::from(r.input_size))
        .collect();
    let per_iter_gas: Vec<f64> = results_high
        .iter()
        .zip(results_low.iter())
        .map(|(h, l)| (h.prover_gas as f64 - l.prover_gas as f64) / iter_delta)
        .collect();
    (depths, per_iter_gas)
}

fn print_raw_table(label: &str, results: &[BenchResult]) {
    println!("\n=== raw measurements ({label}) ===");
    println!(
        "{:>6}  {:>6}  {:>18}  {:>14}  {:>14}",
        "depth", "iters", "prover gas (total)", "total cycles", "region cycles",
    );
    for r in results {
        println!(
            "{:>6}  {:>6}  {:>18}  {:>14}  {:>14}",
            r.input_size, r.iterations, r.prover_gas, r.total_cycles, r.region_cycles,
        );
    }
}

fn print_differential_table(depths: &[f64], per_iter_gas: &[f64]) {
    println!("\n=== differential (clean per-iter prover gas, setup cancelled) ===");
    println!("{:>6}  {:>20}", "depth", "per_iter_gas (clean)");
    for (d, g) in depths.iter().zip(per_iter_gas.iter()) {
        println!("{:>6.0}  {:>20.4}", d, g);
    }
}

fn print_fit(fit: &LinearFit) {
    println!("\n=== linear fit (clean prover gas per call) ===");
    println!("Model: gas_per_call = bias + per_sibling * depth");
    println!("  bias         = {:.2} prover gas / call", fit.bias);
    println!("  per_sibling  = {:.4} prover gas / sibling", fit.per_byte);
    println!("  R²           = {:.6}", fit.r_squared);
    println!("  max residual = {:.2} prover gas", fit.max_residual);
}
