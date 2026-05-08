use std::fs;
use std::path::Path;

use crate::fit::LinearFit;
use crate::BenchResult;

pub struct ReportContext<'a> {
    pub algorithm: &'a str,
    pub sp1_version: &'a str,
    pub git_commit: &'a str,
    pub host_machine: &'a str,
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
    s.push_str(&format!("- Host: {}\n", ctx.host_machine));
    s.push_str("- Method: `ProverClient::execute()` (no proving).\n");
    s.push_str("- Iterations per run: see table.\n\n");

    s.push_str("## Raw measurements\n\n");
    s.push_str("| bytes | iters | prover gas (total) | gas/iter | total cycles | hash_loop cycles | hash cycles/iter |\n");
    s.push_str("|------:|------:|------------------:|--------:|-------------:|-----------------:|----------------:|\n");
    for r in results {
        s.push_str(&format!(
            "| {} | {} | {} | {:.2} | {} | {} | {:.2} |\n",
            r.byte_len,
            r.iterations,
            r.prover_gas,
            r.per_iter_prover_gas(),
            r.total_cycles,
            r.hash_loop_cycles,
            r.per_iter_hash_cycles(),
        ));
    }

    s.push_str("\n## Linear fit: prover gas per hash call\n\n");
    s.push_str("Model: `gas_per_call = bias + per_byte * input_size`.\n\n");
    s.push_str(&format!("- bias (gas / call): **{:.2}**\n", gas_fit.bias));
    s.push_str(&format!(
        "- per_byte (gas / byte): **{:.4}**\n",
        gas_fit.per_byte
    ));
    s.push_str(&format!("- R²: {:.6}\n", gas_fit.r_squared));
    s.push_str(&format!("- max residual: {:.2} gas\n\n", gas_fit.max_residual));

    s.push_str("## Linear fit: RISC-V cycles per hash call (sanity check)\n\n");
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

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out_path, s)?;
    Ok(())
}
