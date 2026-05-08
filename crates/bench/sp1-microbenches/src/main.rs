use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use clap::{Parser, Subcommand};
use sp1_microbenches::fit::{fit_hash_cycles_per_byte, fit_prover_gas_per_byte};
use sp1_microbenches::reports::{write_markdown, ReportContext};
use sp1_microbenches::BenchResult;
use sp1_sdk::blocking::{Elf, Prover, ProverClient, SP1Stdin};

const SP1_SDK_VERSION: &str = "6.1.0";

const GUEST_SHA256_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-sha256/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-sha256",
);

const DEFAULT_ITERATIONS: u32 = 1000;
const DEFAULT_SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

#[derive(Parser, Debug)]
#[command(name = "sp1-microbenches", about = "ZK gas calibration microbenchmarks")]
struct Cli {
    #[command(subcommand)]
    cmd: BenchCmd,
}

#[derive(Subcommand, Debug)]
enum BenchCmd {
    /// Run the SHA-256 prover-gas sweep.
    Sha256 {
        /// Output report path. Defaults to `reports/sha256-{today}.md` next to this crate.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Iterations per execution.
        #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
        iterations: u32,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        BenchCmd::Sha256 { out, iterations } => run_sha256(out, iterations),
    }
}

fn run_sha256(out_path: Option<PathBuf>, iterations: u32) -> anyhow::Result<()> {
    let elf = load_guest_elf(GUEST_SHA256_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results = Vec::with_capacity(DEFAULT_SIZES.len());
    for &size in DEFAULT_SIZES {
        println!("[run] sha256 byte_len={size} iterations={iterations}");
        let mut stdin = SP1Stdin::new();
        stdin.write(&size);
        stdin.write(&iterations);
        let (_pv, report) = client
            .execute(elf.clone(), stdin)
            .run()
            .with_context(|| format!("sp1 execute failed at byte_len={size}"))?;

        let prover_gas = report
            .gas()
            .context("prover gas not available; ProverClient may have disabled gas calculation")?;
        let total_cycles = report.total_instruction_count();
        let hash_loop_cycles = report
            .cycle_tracker
            .get("hash_loop")
            .copied()
            .unwrap_or(0);
        let invocations = report
            .invocation_tracker
            .get("hash_loop")
            .copied()
            .unwrap_or(0);

        results.push(BenchResult {
            byte_len: size,
            iterations,
            prover_gas,
            total_cycles,
            hash_loop_cycles,
            invocations,
        });
    }

    let gas_fit = fit_prover_gas_per_byte(&results)?;
    let cycles_fit = fit_hash_cycles_per_byte(&results)?;

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let out_path = out_path.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("reports")
            .join(format!("sha256-{}.md", date))
    });
    let git_commit = git_short_sha().unwrap_or_else(|| "unknown".to_string());
    let host = host_machine_descriptor();

    let ctx = ReportContext {
        algorithm: "SHA-256 (sp1-patches precompile)",
        sp1_version: SP1_SDK_VERSION,
        git_commit: &git_commit,
        host_machine: &host,
        date: &date,
    };
    write_markdown(&ctx, &results, &gas_fit, &cycles_fit, &out_path)?;

    println!("\n=== summary ===");
    println!(
        "prover gas / hash call: bias={:.2}, per_byte={:.4}, R²={:.4}",
        gas_fit.bias, gas_fit.per_byte, gas_fit.r_squared
    );
    println!(
        "hash_loop cycles / call: bias={:.2}, per_byte={:.4}, R²={:.4}",
        cycles_fit.bias, cycles_fit.per_byte, cycles_fit.r_squared
    );
    println!("report written to: {}", out_path.display());

    Ok(())
}

fn load_guest_elf(path: &str) -> anyhow::Result<Elf> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("guest ELF not found at {path}; did the build script run?"))?;
    if bytes.is_empty() {
        anyhow::bail!("guest ELF at {path} is empty");
    }
    Ok(Elf::from(bytes))
}

fn git_short_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn host_machine_descriptor() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "?".to_string());
    format!("{os}/{arch}, {cpus} logical CPUs")
}
