//! Shared helpers for the native gas-constant microbenches.
//!
//! Each primitive bench sweeps an input size, lets criterion measure native
//! wall-clock per call, then fits `ns = bias + per_byte * size`. The fitted
//! coefficients (in ns) are converted to gas with a fixed scale and charged to
//! a single dimension, `[X, 0]`.
//!
//! ## Gas unit: `1 gas = 0.01 ns`
//!
//! We deliberately make gas finer than a nanosecond (`gas = ns / 0.01 = ns * 100`)
//! so that sub-1ns per-byte costs survive integer rounding. At `1 ns = 1 gas`
//! every cheap per-byte cost (hash ~0.44 ns/byte, etc.) would ceil to `1` —
//! indistinguishable and over-charging large inputs 2-5x. The absolute scale is
//! economically free: the EIP-1559 base fee (`INITIAL_BASE_FEE_PER_GAS`)
//! auto-adjusts, so only `INITIAL_GAS_LIMIT` needs to move with the scale.
//!
//! The benches live in one crate (unlike the SP1 microbenches, which need a
//! separate guest crate per primitive because each is its own zkVM compilation
//! target — not a constraint for native code).

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct Point {
    point_estimate: f64,
}

#[derive(Deserialize)]
struct Estimates {
    mean: Point,
}

/// Workspace `target/` directory (honours `CARGO_TARGET_DIR`).
pub fn target_dir() -> PathBuf {
    // CARGO_TARGET_DIR is optional; falling back to a walk up to the workspace
    // root when it's unset is intentional.
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .map(|p| p.join("target"))
        .unwrap_or_else(|| manifest.join("target"))
}

/// The mean ns criterion recorded for `group/param`.
pub fn read_mean_ns(group: &str, param: impl std::fmt::Display) -> anyhow::Result<f64> {
    let path = target_dir()
        .join("criterion")
        .join(group)
        .join(param.to_string())
        .join("new/estimates.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let est: Estimates = serde_json::from_str(&raw)?;
    Ok(est.mean.point_estimate)
}

/// Nanoseconds per gas unit. `1 gas = 0.01 ns`, i.e. `gas = ns / NS_PER_GAS`.
/// See the module docs for why gas is finer than a nanosecond.
pub const NS_PER_GAS: f64 = 0.01;

/// Convert a native-ns cost to gas at the fixed scale, rounded up (never
/// under-charge), floored at 1.
fn ns_to_gas(ns: f64) -> u64 {
    ((ns / NS_PER_GAS).ceil() as i64).max(1) as u64
}

/// Read a finished size-sweep from criterion, fit `ns = bias + per_byte * size`,
/// and print the suggested two-part constant (`[X, 0]`, the single charged
/// dimension), converting ns to gas at `1 gas = 0.01 ns`.
pub fn report_size_sweep(
    group: &str,
    sizes: &[u64],
    bias_const: &str,
    per_byte_const: &str,
) -> anyhow::Result<()> {
    println!("\n[fit] {group} — reading criterion estimates");
    let mut input_sizes = Vec::with_capacity(sizes.len());
    let mut ns_per_call = Vec::with_capacity(sizes.len());
    for &size in sizes {
        let ns = read_mean_ns(group, size)?;
        println!("  size={size:<7} mean ns/call = {ns:.2}");
        input_sizes.push(size as f64);
        ns_per_call.push(ns);
    }
    let fit = cost_model_fit::fit_linear(&input_sizes, &ns_per_call)?;

    println!("\n=== fit: ns/call = bias + per_byte * size   (1 gas = 0.01 ns) ===");
    println!("  bias         = {:.2} ns", fit.bias);
    println!("  per_byte     = {:.4} ns/byte", fit.per_byte);
    println!("  R²           = {:.6}", fit.r_squared);
    println!("  max residual = {:.2} ns", fit.max_residual);

    println!("\n=== suggested constants.toml values ===");
    println!("  {bias_const} = [{}, 0]", ns_to_gas(fit.bias));
    println!("  {per_byte_const} = [{}, 0]", ns_to_gas(fit.per_byte));
    Ok(())
}
