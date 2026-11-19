use anyhow::Context;
use clap::Args;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::{load_guest_elf, round_at_least_one, BenchResult};
use sov_gas_tools::fit::{fit_linear, LinearFit};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-borsh/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-borsh",
);

const DEFAULT_ITERATIONS: u32 = 100;
const DEFAULT_BASELINE_ITERATIONS: u32 = 10;
const READER_BYTES_SIZES: &[u32] = &[1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];
const READER_COUNT_SIZES: &[u32] = &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512];
const DECODE_VEC_SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

// Mode flags must match guest-borsh/src/main.rs.
const MODE_READER_BYTES: u8 = 0;
const MODE_READER_COUNT: u8 = 1;
const MODE_DECODE_VEC: u8 = 2;

#[derive(Args, Debug)]
pub struct BorshArgs {
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
    #[arg(long, default_value_t = DEFAULT_BASELINE_ITERATIONS)]
    pub baseline_iterations: u32,
}

/// Runs the three borsh sweeps in order; each constant nets out the earlier sweeps' costs.
pub fn run(args: BorshArgs) -> anyhow::Result<()> {
    let BorshArgs {
        iterations,
        baseline_iterations,
    } = args;

    println!("\n========== STEP 1: reader-bytes ==========");
    let fit_bytes = run_sweep(
        "reader-bytes",
        READER_BYTES_SIZES,
        MODE_READER_BYTES,
        iterations,
        baseline_iterations,
        "bytes",
        |stdin, n_bytes| {
            stdin.write(&n_bytes);
            n_bytes
        },
    )?;
    print_fit(&fit_bytes, "byte");
    let per_byte_read = fit_bytes.per_byte;

    println!("\n========== STEP 2: reader-count ==========");
    let fit_reads = run_sweep(
        "reader-count",
        READER_COUNT_SIZES,
        MODE_READER_COUNT,
        iterations,
        baseline_iterations,
        "reads",
        |stdin, n_reads| {
            stdin.write(&n_reads);
            n_reads
        },
    )?;
    print_fit(&fit_reads, "read");
    let per_read_bias = fit_reads.per_byte - per_byte_read;

    println!("\n========== STEP 3: decode-vec ==========");
    let fit_decode = run_sweep(
        "decode-vec",
        DECODE_VEC_SIZES,
        MODE_DECODE_VEC,
        iterations,
        baseline_iterations,
        "bytes",
        |stdin, payload| {
            let data: Vec<u8> = (0..payload).map(|i| (i as u8).wrapping_mul(0xAB)).collect();
            let buf: Vec<u8> = borsh::to_vec(&data).expect("borsh::to_vec of Vec<u8>");
            let buf_len = u32::try_from(buf.len()).expect("buf len fits in u32");
            stdin.write_vec(buf);
            buf_len
        },
    )?;
    print_fit(&fit_decode, "byte");
    // x is buf_len (4-byte prefix included), so the slope already prices the prefix bytes; the
    // intercept only carries the entry cost + the 2 fixed read biases.
    let bias_borsh_deserialization = fit_decode.bias - 2.0 * per_read_bias;

    println!("\n========== SUMMARY ==========");
    println!("Raw fitted values (prover gas):");
    println!("  per_byte_read              = {per_byte_read:.4}");
    println!("  per_read_bias              = {per_read_bias:.4}  (per_read slope - per_byte_read)");
    println!("  bias_borsh_deserialization = {bias_borsh_deserialization:.2}  (decode intercept - 2·per_read_bias)");
    println!(
        "  Sanity: decode-vec slope = {:.4} vs per_byte_read = {per_byte_read:.4}",
        fit_decode.per_byte
    );

    println!(
        "\nSuggested constants.toml values (full value charged to a single dimension, rounded ≥1):"
    );
    let pbr = round_at_least_one(per_byte_read);
    let prb = round_at_least_one(per_read_bias);
    let bbd = round_at_least_one(bias_borsh_deserialization);
    println!("  BORSH_PER_BYTE_READ          = [{pbr}, 0]");
    println!("  BORSH_PER_READ_BIAS          = [{prb}, 0]");
    println!("  BIAS_BORSH_DESERIALIZATION   = [{bbd}, 0]");

    Ok(())
}

/// Runs the high/low-iteration sweep, differences out per-execution setup, fits a line, and
/// prints the tables. `write_payload` writes each point's stdin payload and returns its input size.
fn run_sweep<W>(
    name: &str,
    sizes: &[u32],
    mode: u8,
    iterations: u32,
    baseline_iterations: u32,
    x_label: &str,
    mut write_payload: W,
) -> anyhow::Result<LinearFit>
where
    W: FnMut(&mut SP1Stdin, u32) -> u32,
{
    anyhow::ensure!(
        iterations > baseline_iterations,
        "--iterations must be greater than --baseline-iterations"
    );

    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results_high = Vec::with_capacity(sizes.len());
    let mut results_low = Vec::with_capacity(sizes.len());
    for &n in sizes {
        for (iter_count, results) in [
            (iterations, &mut results_high),
            (baseline_iterations, &mut results_low),
        ] {
            println!("[run] borsh {name} {x_label}={n} iterations={iter_count}");
            let mut stdin = SP1Stdin::new();
            stdin.write(&mode);
            stdin.write(&iter_count);
            let input_size = write_payload(&mut stdin, n);
            let report =
                execute_and_collect(&client, elf.clone(), stdin, input_size, iter_count)
                    .with_context(|| format!("{name} failed at {x_label}={n} iter={iter_count}"))?;
            results.push(report);
        }
    }

    let (sizes_f, per_iter_gas) =
        differential(&results_high, &results_low, iterations, baseline_iterations);
    let fit = fit_linear(&sizes_f, &per_iter_gas)?;

    print_raw_table("high-iter", &results_high, x_label);
    print_raw_table("low-iter (baseline)", &results_low, x_label);
    print_differential_table(&sizes_f, &per_iter_gas, x_label);

    Ok(fit)
}

fn execute_and_collect(
    client: &impl Prover,
    elf: sp1_sdk::blocking::Elf,
    stdin: SP1Stdin,
    input_size: u32,
    iterations: u32,
) -> anyhow::Result<BenchResult> {
    let (_pv, report) = client.execute(elf, stdin).run()?;
    let prover_gas = report
        .gas()
        .context("prover gas not available; ProverClient may have disabled gas calculation")?;
    let total_cycles = report.total_instruction_count();
    let region_cycles = report.cycle_tracker.get("borsh_loop").copied().unwrap_or(0);
    Ok(BenchResult {
        input_size,
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
    let sizes: Vec<f64> = results_high
        .iter()
        .map(|r| f64::from(r.input_size))
        .collect();
    let per_iter_gas: Vec<f64> = results_high
        .iter()
        .zip(results_low.iter())
        .map(|(h, l)| (h.prover_gas as f64 - l.prover_gas as f64) / iter_delta)
        .collect();
    (sizes, per_iter_gas)
}

fn print_raw_table(label: &str, results: &[BenchResult], x_label: &str) {
    println!("\n=== raw measurements ({label}) ===");
    println!(
        "{:>6}  {:>6}  {:>18}  {:>14}  {:>14}",
        x_label, "iters", "prover gas (total)", "total cycles", "region cycles",
    );
    for r in results {
        println!(
            "{:>6}  {:>6}  {:>18}  {:>14}  {:>14}",
            r.input_size, r.iterations, r.prover_gas, r.total_cycles, r.region_cycles,
        );
    }
}

fn print_differential_table(sizes: &[f64], per_iter_gas: &[f64], x_label: &str) {
    println!("\n=== differential (clean per-iter prover gas, setup cancelled) ===");
    println!("{:>6}  {:>20}", x_label, "per_iter_gas (clean)");
    for (s, g) in sizes.iter().zip(per_iter_gas.iter()) {
        println!("{:>6.0}  {:>20.4}", s, g);
    }
}

fn print_fit(fit: &LinearFit, unit: &str) {
    println!("\n=== linear fit (clean prover gas per call) ===");
    println!("Model: gas_per_call = bias + per_{unit} * input_size");
    println!("  bias         = {:.2} prover gas / call", fit.bias);
    println!("  per_{unit}     = {:.4} prover gas / {unit}", fit.per_byte);
    println!("  R²           = {:.6}", fit.r_squared);
    println!("  max residual = {:.2} prover gas", fit.max_residual);
}
