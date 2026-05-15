use anyhow::Context;
use clap::{Args, Subcommand};
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::fit::{fit_prover_gas_per_byte, LinearFit};
use crate::{load_guest_elf, BenchResult};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-borsh/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-borsh",
);

const DEFAULT_ITERATIONS: u32 = 100;
const READER_BYTES_SIZES: &[u32] = &[1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];
const READER_COUNT_SIZES: &[u32] = &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512];
const DECODE_VEC_SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

// Mode flags must match guest-borsh/src/main.rs.
const MODE_READER_BYTES: u8 = 0;
const MODE_READER_COUNT: u8 = 1;
const MODE_DECODE_VEC: u8 = 2;

#[derive(Args, Debug)]
pub struct BorshArgs {
    #[command(subcommand)]
    cmd: BorshSubcommand,
}

#[derive(Subcommand, Debug)]
enum BorshSubcommand {
    /// Sweep buffer size at 1 read per iter. Calibrates `BORSH_PER_BYTE_READ`.
    ReaderBytes(ReaderBytesArgs),
    /// Sweep read count at 1 byte per read. Calibrates `BORSH_PER_READ_BIAS`.
    ReaderCount(ReaderCountArgs),
    /// Sweep `Vec<u8>` decode size. Calibrates `BIAS_BORSH_DESERIALIZATION`.
    DecodeVec(DecodeVecArgs),
}

#[derive(Args, Debug)]
pub struct ReaderBytesArgs {
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
}

#[derive(Args, Debug)]
pub struct ReaderCountArgs {
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
    /// Slope from `borsh reader-bytes`. If provided, prints calibrated `BORSH_PER_READ_BIAS`.
    #[arg(long)]
    pub per_byte_read: Option<f64>,
}

#[derive(Args, Debug)]
pub struct DecodeVecArgs {
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
    /// Slope from `borsh reader-bytes`.
    #[arg(long)]
    pub per_byte_read: Option<f64>,
    /// Calibrated `BORSH_PER_READ_BIAS` (from `borsh reader-count` minus `per_byte_read`).
    #[arg(long)]
    pub per_read_bias: Option<f64>,
}

impl BorshArgs {
    pub fn run(self) -> anyhow::Result<()> {
        match self.cmd {
            BorshSubcommand::ReaderBytes(args) => run_reader_bytes(args),
            BorshSubcommand::ReaderCount(args) => run_reader_count(args),
            BorshSubcommand::DecodeVec(args) => run_decode_vec(args),
        }
    }
}

fn run_reader_bytes(args: ReaderBytesArgs) -> anyhow::Result<()> {
    let ReaderBytesArgs { iterations } = args;
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results = Vec::with_capacity(READER_BYTES_SIZES.len());
    for &n_bytes in READER_BYTES_SIZES {
        println!("[run] borsh reader-bytes n_bytes={n_bytes} iterations={iterations}");

        let mut stdin = SP1Stdin::new();
        stdin.write(&MODE_READER_BYTES);
        stdin.write(&iterations);
        stdin.write(&n_bytes);

        let report = execute_and_collect(&client, elf.clone(), stdin, n_bytes, iterations)
            .with_context(|| format!("reader-bytes failed at n_bytes={n_bytes}"))?;
        results.push(report);
    }

    let fit = fit_prover_gas_per_byte(&results)?;
    print_table(&results, "bytes");
    print_fit_per_byte(&fit);

    println!("\n=== suggested constant ===");
    println!(
        "  BORSH_PER_BYTE_READ[1] ≈ {}",
        fit.per_byte.round() as i64
    );
    Ok(())
}

fn run_reader_count(args: ReaderCountArgs) -> anyhow::Result<()> {
    let ReaderCountArgs {
        iterations,
        per_byte_read,
    } = args;
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results = Vec::with_capacity(READER_COUNT_SIZES.len());
    for &n_reads in READER_COUNT_SIZES {
        println!("[run] borsh reader-count n_reads={n_reads} iterations={iterations}");

        let mut stdin = SP1Stdin::new();
        stdin.write(&MODE_READER_COUNT);
        stdin.write(&iterations);
        stdin.write(&n_reads);

        let report = execute_and_collect(&client, elf.clone(), stdin, n_reads, iterations)
            .with_context(|| format!("reader-count failed at n_reads={n_reads}"))?;
        results.push(report);
    }

    let fit = fit_prover_gas_per_byte(&results)?;
    print_table(&results, "reads");
    println!("\n=== linear fit (prover gas per call) ===");
    println!("Model: gas_per_call = bias + per_read * n_reads");
    println!("  bias         = {:.2} prover gas / call", fit.bias);
    println!("  per_read     = {:.4} prover gas / read", fit.per_byte);
    println!("  R²           = {:.6}", fit.r_squared);
    println!("  max residual = {:.2} prover gas", fit.max_residual);

    println!("\n=== suggested constant ===");
    match per_byte_read {
        Some(per_byte) => {
            let bias = fit.per_byte - per_byte;
            println!(
                "  Using per_byte_read = {per_byte:.4} from `borsh reader-bytes`:"
            );
            println!(
                "  BORSH_PER_READ_BIAS[1] ≈ per_read - per_byte_read = {} - {} = {}",
                fit.per_byte.round() as i64,
                per_byte.round() as i64,
                bias.round() as i64
            );
        }
        None => {
            println!(
                "  Re-run with `--per-byte-read <slope>` from `borsh reader-bytes` to compute"
            );
            println!(
                "  BORSH_PER_READ_BIAS = per_read - per_byte_read (currently per_read = {:.4})",
                fit.per_byte
            );
        }
    }
    Ok(())
}

fn run_decode_vec(args: DecodeVecArgs) -> anyhow::Result<()> {
    let DecodeVecArgs {
        iterations,
        per_byte_read,
        per_read_bias,
    } = args;
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results = Vec::with_capacity(DECODE_VEC_SIZES.len());
    for &payload in DECODE_VEC_SIZES {
        println!("[run] borsh decode-vec payload={payload} iterations={iterations}");

        let data: Vec<u8> = (0..payload).map(|i| (i as u8).wrapping_mul(0xAB)).collect();
        let buf: Vec<u8> = borsh::to_vec(&data).expect("borsh::to_vec of Vec<u8>");

        let mut stdin = SP1Stdin::new();
        stdin.write(&MODE_DECODE_VEC);
        stdin.write(&iterations);
        stdin.write_vec(buf.clone());

        let buf_len = u32::try_from(buf.len()).expect("buf len fits in u32");
        let report = execute_and_collect(&client, elf.clone(), stdin, buf_len, iterations)
            .with_context(|| format!("decode-vec failed at payload={payload}"))?;
        results.push(report);
    }

    let fit = fit_prover_gas_per_byte(&results)?;
    print_table(&results, "bytes");
    print_fit_per_byte(&fit);

    println!("\n=== suggested constant ===");
    match (per_byte_read, per_read_bias) {
        (Some(pb), Some(prb)) => {
            // Vec<u8> decode = 2 reads (length + body). Intercept includes bias_borsh_deserialization
            // + 2 * per_read_bias + 4 * per_byte_read (length prefix is 4 bytes).
            let bias_const = fit.bias - 2.0 * prb - 4.0 * pb;
            println!(
                "  Using per_byte_read = {pb:.4}, per_read_bias = {prb:.4}:"
            );
            println!(
                "  BIAS_BORSH_DESERIALIZATION[1] ≈ intercept - 2·per_read_bias - 4·per_byte_read"
            );
            println!(
                "                              ≈ {:.2} - {:.2} - {:.2} = {}",
                fit.bias,
                2.0 * prb,
                4.0 * pb,
                bias_const.round() as i64
            );
            println!(
                "  Sanity: fit slope = {:.4} should match per_byte_read = {pb:.4}",
                fit.per_byte
            );
        }
        _ => {
            println!(
                "  Re-run with `--per-byte-read <X> --per-read-bias <Y>` (from reader-bytes / reader-count)"
            );
            println!(
                "  to compute BIAS_BORSH_DESERIALIZATION = intercept - 2·per_read_bias - 4·per_byte_read"
            );
            println!("  (currently intercept = {:.2})", fit.bias);
        }
    }
    Ok(())
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
    let region_cycles = report
        .cycle_tracker
        .get("borsh_loop")
        .copied()
        .unwrap_or(0);
    let invocations = report
        .invocation_tracker
        .get("borsh_loop")
        .copied()
        .unwrap_or(0);

    Ok(BenchResult {
        input_size,
        iterations,
        prover_gas,
        total_cycles,
        region_cycles,
        invocations,
    })
}

fn print_table(results: &[BenchResult], x_label: &str) {
    println!("\n=== raw measurements ===");
    println!(
        "{:>6}  {:>6}  {:>18}  {:>10}  {:>14}  {:>14}  {:>14}",
        x_label, "iters", "prover gas (total)", "gas/iter", "total cycles", "region cycles", "region/iter",
    );
    for r in results {
        println!(
            "{:>6}  {:>6}  {:>18}  {:>10.2}  {:>14}  {:>14}  {:>14.2}",
            r.input_size,
            r.iterations,
            r.prover_gas,
            r.per_iter_prover_gas(),
            r.total_cycles,
            r.region_cycles,
            r.per_iter_region_cycles(),
        );
    }
}

fn print_fit_per_byte(fit: &LinearFit) {
    println!("\n=== linear fit (prover gas per call) ===");
    println!("Model: gas_per_call = bias + per_byte * input_size");
    println!("  bias         = {:.2} prover gas / call", fit.bias);
    println!("  per_byte     = {:.4} prover gas / byte", fit.per_byte);
    println!("  R²           = {:.6}", fit.r_squared);
    println!("  max residual = {:.2} prover gas", fit.max_residual);
}
