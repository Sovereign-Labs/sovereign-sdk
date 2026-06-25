//! Helpers for the native wall-clock gas microbenches: read criterion's per-size
//! estimates, convert ns → gas, and print suggested `constants.toml` values.
//!
//! ## Gas unit: `1 gas = 0.01 ns`
//!
//! We deliberately make gas finer than a nanosecond (`gas = ns / 0.01 = ns * 100`)
//! so that sub-1ns per-byte costs survive integer rounding. At `1 ns = 1 gas`
//! every cheap per-byte cost (hash ~0.44 ns/byte, etc.) would ceil to `1` —
//! indistinguishable and over-charging large inputs 2-5x.

use std::path::PathBuf;

use serde::Deserialize;

use crate::fit::fit_linear;

#[derive(Deserialize)]
struct Point {
    point_estimate: f64,
}

#[derive(Deserialize)]
struct Estimates {
    mean: Point,
}

pub fn target_dir() -> PathBuf {
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
///
/// The constants suggestion assumes a fresh, full size sweep. Criterion does not
/// remove old `new/estimates.json` files during filtered runs, so delete the
/// group's `target/criterion/<group>/` directory before relying on suggestions
/// after running with a filter.
pub fn report_size_sweep(
    group: &str,
    sizes: &[u64],
    bias_const: &str,
    per_byte_const: &str,
) -> anyhow::Result<()> {
    println!("\n[fit] {group} — reading criterion estimates");
    eprintln!(
        "warn: constants suggestion assumes a fresh full sweep; if you ran a filtered \
         benchmark, delete {} before relying on this output",
        target_dir().join("criterion").join(group).display()
    );
    let mut input_sizes = Vec::with_capacity(sizes.len());
    let mut ns_per_call = Vec::with_capacity(sizes.len());
    for &size in sizes {
        let ns = read_mean_ns(group, size)?;
        println!("  size={size:<7} mean ns/call = {ns:.2}");
        input_sizes.push(size as f64);
        ns_per_call.push(ns);
    }
    let fit = fit_linear(&input_sizes, &ns_per_call)?;

    if fit.bias < 0.0 || fit.per_byte < 0.0 {
        eprintln!(
            "warn: degenerate fit (bias={:.2} ns, per_byte={:.4} ns/byte) — a negative \
             coefficient floors to 1 gas; the suggested constant is NOT meaningful. \
             Re-run with more samples / a wider size sweep.",
            fit.bias, fit.per_byte
        );
    }

    // Base = directly-measured fixed cost (size=0), not the fitted intercept.
    // Over a wide size range the OLS intercept is high-leverage and unstable
    // (it swung 29-131 ns across runs while size=0 stayed ~56-60 ns) and can
    // undercharge; the size=0 point is stable
    let base_ns = match sizes.iter().position(|&s| s == 0) {
        Some(i) => ns_per_call[i],
        None => {
            eprintln!(
                "warn: no size=0 point in sweep; using the fitted intercept \
                 ({:.2} ns) as the base, which can be unstable.",
                fit.bias
            );
            fit.bias
        }
    };

    println!("\n=== fit: ns/call = bias + per_byte * size   (1 gas = 0.01 ns) ===");
    println!(
        "  fitted bias   = {:.2} ns  (intercept; informational)",
        fit.bias
    );
    println!(
        "  base (size=0) = {:.2} ns  (used for the constant)",
        base_ns
    );
    println!("  per_byte      = {:.4} ns/byte", fit.per_byte);
    println!("  R²            = {:.6}", fit.r_squared);
    println!("  max residual  = {:.2} ns", fit.max_residual);

    println!("\n=== suggested constants.toml values ===");
    println!("  {bias_const} = [{}, 0]", ns_to_gas(base_ns));
    println!("  {per_byte_const} = [{}, 0]", ns_to_gas(fit.per_byte));
    Ok(())
}

/// Read a finished sweep whose x-axis is an operation *count* (not bytes), fit
/// `ns = fixed + per_op * count`, and suggest the per-op constant from the
/// **slope**. Use this when a fixed per-batch overhead (e.g. one block commit's
/// fsync) must be excluded from the per-op charge: it is the fit intercept and
/// drops out of the slope. The intercept is printed for information only.
pub fn report_marginal_sweep(
    group: &str,
    counts: &[u64],
    per_op_const: &str,
) -> anyhow::Result<()> {
    println!("\n[fit] {group} — reading criterion estimates (marginal per-op)");
    let mut counts_f = Vec::with_capacity(counts.len());
    let mut ns_per_batch = Vec::with_capacity(counts.len());
    for &count in counts {
        let ns = read_mean_ns(group, count)?;
        println!("  count={count:<7} mean ns/batch = {ns:.2}");
        counts_f.push(count as f64);
        ns_per_batch.push(ns);
    }
    let fit = fit_linear(&counts_f, &ns_per_batch)?;

    if fit.per_byte < 0.0 {
        eprintln!(
            "warn: negative slope ({:.4} ns/op) — no marginal signal; the suggested \
             constant floors to 1 gas and is NOT meaningful. Widen the count sweep / \
             add samples.",
            fit.per_byte
        );
    }

    println!("\n=== fit: ns/batch = fixed + per_op * count   (1 gas = 0.01 ns) ===");
    println!("  per_op (slope)    = {:.4} ns/op    (used for the constant)", fit.per_byte);
    println!(
        "  fixed (intercept) = {:.2} ns/batch  (per-commit overhead; NOT charged)",
        fit.bias
    );
    println!("  R²                = {:.6}", fit.r_squared);
    println!("  max residual      = {:.2} ns", fit.max_residual);

    println!("\n=== suggested constants.toml value ===");
    println!("  {per_op_const} = [{}, 0]", ns_to_gas(fit.per_byte));
    Ok(())
}
