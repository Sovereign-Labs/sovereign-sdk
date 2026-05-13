use anyhow::Context;
use clap::Args;
use sov_rollup_interface::crypto::PrivateKey;
use sov_sp1_adapter::crypto::private_key::SP1PrivateKey;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::fit::fit_prover_gas_per_byte;
use crate::{load_guest_elf, BenchResult};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-ed25519/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-ed25519",
);

const DEFAULT_ITERATIONS: u32 = 100;
const SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

#[derive(Args, Debug)]
pub struct Ed25519Args {
    /// Iterations per execution.
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
}

pub fn run(args: Ed25519Args) -> anyhow::Result<()> {
    let Ed25519Args { iterations } = args;
    let elf = load_guest_elf(GUEST_ELF_PATH)?;
    let client = ProverClient::from_env();

    let private_key = SP1PrivateKey::generate();
    let pubkey_bytes: Vec<u8> = private_key.pub_key().bytes().to_vec();

    let mut results = Vec::with_capacity(SIZES.len());
    for &size in SIZES {
        println!("[run] ed25519 byte_len={size} iterations={iterations}");

        let msg: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(0xAB)).collect();
        let sig_bytes: Vec<u8> = private_key.sign(&msg).as_ref().to_vec();

        let mut stdin = SP1Stdin::new();
        stdin.write_vec(pubkey_bytes.clone());
        stdin.write_vec(sig_bytes);
        stdin.write_vec(msg);
        stdin.write(&iterations);

        let (_pv, report) = client
            .execute(elf.clone(), stdin)
            .run()
            .with_context(|| format!("sp1 execute failed at byte_len={size}"))?;

        let prover_gas = report
            .gas()
            .context("prover gas not available; ProverClient may have disabled gas calculation")?;
        let total_cycles = report.total_instruction_count();
        let region_cycles = report
            .cycle_tracker
            .get("verify_loop")
            .copied()
            .unwrap_or(0);
        let invocations = report
            .invocation_tracker
            .get("verify_loop")
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
        "  DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION[1]    ≈ {}",
        gas_fit.bias.round() as i64
    );
    println!(
        "  DEFAULT_GAS_TO_CHARGE_PER_BYTE_SIGNATURE_VERIFICATION[1]     ≈ {}",
        gas_fit.per_byte.round() as i64
    );

    Ok(())
}
