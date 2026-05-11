use anyhow::Context;
use clap::Args;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::fit::fit_prover_gas_per_byte;
use crate::{load_guest_elf, BenchResult};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-sha256/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-sha256",
);

const DEFAULT_ITERATIONS: u32 = 1000;
const SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

#[derive(Args, Debug)]
pub struct Sha256Args {
    /// Iterations per execution.
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
}

pub fn run(args: Sha256Args) -> anyhow::Result<()> {
    let Sha256Args { iterations } = args;
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

    println!("\n=== raw measurements ===");
    println!(
        "{:>6}  {:>6}  {:>18}  {:>10}  {:>14}  {:>14}  {:>14}",
        "bytes",
        "iters",
        "prover gas (total)",
        "gas/iter",
        "total cycles",
        "region cycles",
        "region/iter",
    );
    for r in &results {
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

    println!("\n=== linear fit (prover gas per call) ===");
    println!("Model: gas_per_call = bias + per_byte * input_size");
    println!("  bias         = {:.2} prover gas / call", gas_fit.bias);
    println!("  per_byte     = {:.4} prover gas / byte", gas_fit.per_byte);
    println!("  R²           = {:.6}", gas_fit.r_squared);
    println!("  max residual = {:.2} prover gas", gas_fit.max_residual);

    println!("\n=== suggested constants.toml values (raw SP1 prover gas, 1:1) ===");
    println!(
        "  GAS_TO_CHARGE_HASH_UPDATE[1]          ≈ {}",
        gas_fit.bias.round() as i64
    );
    println!(
        "  GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE[1] ≈ {}",
        gas_fit.per_byte.round() as i64
    );

    Ok(())
}
