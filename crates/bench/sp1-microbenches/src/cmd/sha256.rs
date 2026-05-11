use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::fit::{fit_prover_gas_per_byte, fit_region_cycles_per_byte};
use crate::reports::{write_markdown, ReportContext};
use crate::{git_short_sha, load_guest_elf, BenchResult, SP1_SDK_VERSION};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-sha256/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-sha256",
);

const DEFAULT_ITERATIONS: u32 = 1000;
const SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

const ALGORITHM_DESCRIPTION: &str =
    "SHA-256 via MeteredHasher + UnlimitedGasMeter (S::CryptoSpec::Hasher = sha2::Sha256, sp1-patches precompile)";

#[derive(Args, Debug)]
pub struct Sha256Args {
    /// Output report path. Defaults to `reports/sha256-{today}.md` next to this crate.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Iterations per execution.
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
}

pub fn run(args: Sha256Args) -> anyhow::Result<()> {
    let Sha256Args { out, iterations } = args;
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let mut results = Vec::with_capacity(SIZES.len());
    for &size in SIZES {
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
        let region_cycles = report.cycle_tracker.get("hash_loop").copied().unwrap_or(0);
        let invocations = report
            .invocation_tracker
            .get("hash_loop")
            .copied()
            .unwrap_or(0);

        results.push(BenchResult {
            input_size: size,
            iterations,
            prover_gas,
            total_cycles,
            region_cycles,
            invocations,
        });
    }

    let gas_fit = fit_prover_gas_per_byte(&results)?;
    let cycles_fit = fit_region_cycles_per_byte(&results)?;

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let out_path = out.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("reports")
            .join(format!("sha256-{}.md", date))
    });
    let git_commit = git_short_sha().unwrap_or_else(|| "unknown".to_string());

    let ctx = ReportContext {
        algorithm: ALGORITHM_DESCRIPTION,
        sp1_version: SP1_SDK_VERSION,
        git_commit: &git_commit,
        date: &date,
    };
    write_markdown(&ctx, &results, &gas_fit, &cycles_fit, &out_path)?;

    println!("\n=== summary ===");
    println!(
        "prover gas / call: bias={:.2}, per_byte={:.4}, R²={:.4}",
        gas_fit.bias, gas_fit.per_byte, gas_fit.r_squared
    );
    println!(
        "region cycles / call: bias={:.2}, per_byte={:.4}, R²={:.4}",
        cycles_fit.bias, cycles_fit.per_byte, cycles_fit.r_squared
    );
    println!("report written to: {}", out_path.display());

    Ok(())
}
