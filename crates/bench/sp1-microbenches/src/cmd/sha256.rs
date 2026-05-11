use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use sp1_sdk::blocking::{Prover, ProverClient, SP1Stdin};

use crate::fit::{fit_prover_gas_per_byte, fit_region_cycles_per_byte, LinearFit};
use crate::reports::{render_markdown, BenchOutput, ReportContent};
use crate::{load_guest_elf, today, BenchResult};

const GUEST_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/guest-sha256/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-microbench-guest-sha256",
);

const DEFAULT_ITERATIONS: u32 = 1000;
const SIZES: &[u32] = &[0, 1, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536];

pub struct Sha256Bench;

impl ReportContent for Sha256Bench {
    fn algorithm(&self) -> &str {
        "SHA-256 via MeteredHasher + UnlimitedGasMeter (S::CryptoSpec::Hasher = sha2::Sha256, sp1-patches precompile)"
    }

    fn scope(&self) -> &str {
        "These constants are charged by `MeteredHasher::digest` and therefore apply only to API-level hashing — transaction-hash calculation, credential-id derivation, and similar call sites. Jellyfish-Merkle-Tree internal-node hashing uses the **raw** `S::Hasher` and is **not** governed by these constants; its proving-time cost is absorbed by the storage-access gas constants instead."
    }

    fn suggested_constants(&self, gas_fit: &LinearFit) -> String {
        format!(
            "- `GAS_TO_CHARGE_HASH_UPDATE[1]` ≈ **{}**\n- `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE[1]` ≈ **{}**",
            gas_fit.bias.round() as i64,
            gas_fit.per_byte.round() as i64,
        )
    }

    fn extra_glossary(&self) -> &'static [(&'static str, &'static str)] {
        &[
            (
                "call",
                "One invocation of `MeteredHasher::digest(&data, &mut meter)` in the guest's hot loop. Each call internally invokes `MeteredHasher::update` exactly once, which is where the rollup's gas charges are applied (one bias charge plus one linear-per-byte charge).",
            ),
            (
                "`MeteredHasher`",
                "SDK wrapper at `crates/module-system/sov-modules-api/src/gas/metered_utils.rs` that charges gas before delegating to the underlying `Digest` impl. The production call site is `calculate_hash_metered` in `crates/module-system/sov-modules-api/src/runtime/capabilities/authentication.rs`, which is what this microbench mirrors.",
            ),
        ]
    }
}

#[derive(Args, Debug)]
pub struct Sha256Args {
    /// Output report path. Defaults to `reports/sha256-{today}.md` next to this crate.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Iterations per execution.
    #[arg(long, default_value_t = DEFAULT_ITERATIONS)]
    pub iterations: u32,
}

pub fn run(args: Sha256Args) -> anyhow::Result<BenchOutput> {
    let Sha256Args { out: _, iterations } = args;
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

    let markdown = render_markdown(&Sha256Bench, &results, &gas_fit, &cycles_fit);

    let summary = format!(
        "\n=== summary ===\nprover gas / call: bias={:.2}, per_byte={:.4}, R²={:.4}\nregion cycles / call: bias={:.2}, per_byte={:.4}, R²={:.4}",
        gas_fit.bias,
        gas_fit.per_byte,
        gas_fit.r_squared,
        cycles_fit.bias,
        cycles_fit.per_byte,
        cycles_fit.r_squared,
    );

    Ok(BenchOutput {
        markdown,
        default_filename: format!("sha256-{}.md", today()),
        summary,
    })
}
