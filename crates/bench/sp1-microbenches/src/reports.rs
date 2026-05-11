use std::fs;
use std::path::Path;

use crate::fit::LinearFit;
use crate::BenchResult;

pub struct ReportContext<'a> {
    pub algorithm: &'a str,
    pub sp1_version: &'a str,
    pub git_commit: &'a str,
    pub date: &'a str,
}

pub fn write_markdown(
    ctx: &ReportContext<'_>,
    results: &[BenchResult],
    gas_fit: &LinearFit,
    cycles_fit: &LinearFit,
    out_path: &Path,
) -> anyhow::Result<()> {
    let mut s = String::new();
    s.push_str(&format!("# {} prover-gas microbench\n\n", ctx.algorithm));
    s.push_str(&format!("- Date: {}\n", ctx.date));
    s.push_str(&format!("- SP1 SDK: {}\n", ctx.sp1_version));
    s.push_str(&format!("- Git commit: {}\n", ctx.git_commit));
    s.push_str("- Method: `ProverClient::execute()` (no proving).\n");
    s.push_str("- Iterations per run: see table.\n");
    s.push_str(
        "- Term definitions: see [Appendix: Glossary](#appendix-glossary) at the bottom.\n\n",
    );

    s.push_str("## Raw measurements\n\n");
    s.push_str("| bytes | iters | prover gas (total) | gas/iter | total cycles | region cycles | region cycles/iter |\n");
    s.push_str("|------:|------:|------------------:|--------:|-------------:|--------------:|------------------:|\n");
    for r in results {
        s.push_str(&format!(
            "| {} | {} | {} | {:.2} | {} | {} | {:.2} |\n",
            r.input_size,
            r.iterations,
            r.prover_gas,
            r.per_iter_prover_gas(),
            r.total_cycles,
            r.region_cycles,
            r.per_iter_region_cycles(),
        ));
    }

    s.push_str("\n## Linear fit: prover gas per call\n\n");
    s.push_str("Model: `gas_per_call = bias + per_byte * input_size`.\n\n");
    s.push_str(&format!("- bias (gas / call): **{:.2}**\n", gas_fit.bias));
    s.push_str(&format!(
        "- per_byte (gas / byte): **{:.4}**\n",
        gas_fit.per_byte
    ));
    s.push_str(&format!("- R²: {:.6}\n", gas_fit.r_squared));
    s.push_str(&format!(
        "- max residual: {:.2} gas\n\n",
        gas_fit.max_residual
    ));

    s.push_str("## Linear fit: RISC-V cycles per call (sanity check)\n\n");
    s.push_str(&format!("- bias (cycles / call): {:.2}\n", cycles_fit.bias));
    s.push_str(&format!(
        "- per_byte (cycles / byte): {:.4}\n",
        cycles_fit.per_byte
    ));
    s.push_str(&format!("- R²: {:.6}\n", cycles_fit.r_squared));
    s.push_str(&format!(
        "- max residual: {:.2} cycles\n\n",
        cycles_fit.max_residual
    ));

    s.push_str("## Scope\n\n");
    s.push_str("These constants are charged by `MeteredHasher::digest` and therefore apply only to API-level hashing — transaction-hash calculation, credential-id derivation, and similar call sites. Jellyfish-Merkle-Tree internal-node hashing uses the **raw** `S::Hasher` and is **not** governed by these constants; its proving-time cost is absorbed by the storage-access gas constants instead.\n\n");

    s.push_str("## Suggested ZK gas constants\n\n");
    s.push_str("Raw values (rounded). Apply a global scaling factor when wiring into `constants.toml` so that all metered primitives share a coherent unit.\n\n");
    s.push_str(&format!(
        "- `GAS_TO_CHARGE_HASH_UPDATE[1]` ≈ **{}**\n",
        gas_fit.bias.round() as i64
    ));
    s.push_str(&format!(
        "- `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE[1]` ≈ **{}**\n",
        gas_fit.per_byte.round() as i64
    ));

    s.push_str("\n## Appendix: Glossary\n\n");
    s.push_str("**prover gas** — SP1's `ExecutionReport.gas()` value. A predictor of GPU proving time computed from the execution trace's per-shard padded heights, normalized so it's of similar magnitude to RISC-V cycles. *Not* the same as the rollup's application gas in `constants.toml`; mapping prover gas to application gas requires picking a global scaling factor once all primitives are calibrated.\n\n");
    s.push_str("**RISC-V cycles** — Count of RISC-V instructions executed by the SP1 zkVM. Easy to reason about but a worse predictor of proving cost than prover gas because precompile cycles (e.g. SHA-256 ecall) are far cheaper to prove per-cycle than ordinary cycles. Reported as a sanity check.\n\n");
    s.push_str("**call** — One invocation of `MeteredHasher::digest(&data, &mut meter)` in the guest's hot loop. Each call internally invokes `MeteredHasher::update` exactly once, which is where the rollup's gas charges are applied (one bias charge plus one linear-per-byte charge).\n\n");
    s.push_str("**iter / iters** — Loop iterations of the inner benchmark loop. We run many iterations per execution to amortize the per-program startup cost of the SP1 executor over the operation under measurement. `gas/iter = prover_gas_total / iters`.\n\n");
    s.push_str("**bias** — The intercept of the linear fit, expressed in *prover gas per call*. This is the fixed cost paid every time `MeteredHasher::update` runs, regardless of input size: precompile syscall setup, padding, finalization, wrapper overhead. Maps directly to `GAS_TO_CHARGE_HASH_UPDATE[1]` in `constants.toml`.\n\n");
    s.push_str("**per_byte** — The slope of the linear fit, in *prover gas per byte of input*. The marginal cost of one additional input byte. Maps directly to `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE[1]`.\n\n");
    s.push_str("**R²** — Coefficient of determination for the linear fit. 1.0 means the linear model perfectly explains the variance in the measurements; 0.0 means no relationship. Anything below ~0.99 across an input-size sweep means the operation is not well-modeled as `bias + per_byte × size` and needs a richer cost function.\n\n");
    s.push_str("**max residual** — The largest single-point deviation between measured cost and the linear-fit prediction, in the fit's units. With R² near 1 this is the worst-case error of using the linear model — useful for spotting step-function structure (SHA-256 processes 64-byte blocks, so residuals oscillate at small sizes and shrink as inputs grow).\n\n");
    s.push_str("**region cycles** — Cycles measured inside the labeled `cycle-tracker-report-{start,end}` markers in the guest (for SHA-256 the label is `hash_loop`). Captures only the inner work loop, excluding program startup and input reading. Available because SP1 exposes per-region cycles via `ExecutionReport.cycle_tracker`. No equivalent exists for prover gas, which is whole-execution-only — hence the isolated microbench design.\n\n");
    s.push_str("**total cycles** — Whole-program cycle count (`ExecutionReport.total_instruction_count()`). Includes startup, input reading, loop overhead, commit, and the work itself. Always larger than `region cycles`.\n\n");
    s.push_str("**`MeteredHasher`** — SDK wrapper at `crates/module-system/sov-modules-api/src/gas/metered_utils.rs` that charges gas before delegating to the underlying `Digest` impl. The production call site is `calculate_hash_metered` in `crates/module-system/sov-modules-api/src/runtime/capabilities/authentication.rs`, which is what this microbench mirrors.\n\n");
    s.push_str("**`UnlimitedGasMeter`** — Stateless gas meter (`PhantomData<S>`) whose `charge_gas` and `charge_linear_gas` are no-ops via the `GasMeter` trait's default impls. Used here so the metering wrapper compiles in but inlines to zero work — we measure the hash, not the meter arithmetic.\n\n");

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out_path, s)?;
    Ok(())
}
