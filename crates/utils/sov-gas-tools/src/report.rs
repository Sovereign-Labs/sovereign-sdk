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
pub fn ns_to_gas(ns: f64) -> u64 {
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

/// Read a finished single-parameter sweep, fit `ns = fixed + slope * x`, and
/// suggest the constant from the **slope**. The fixed intercept (e.g. a
/// once-per-commit fsync) is discarded — use this when only the marginal,
/// per-unit cost should be charged. Benchmark ids must be the bare `x` values.
pub fn report_slope_sweep(group: &str, xs: &[u64], slope_const: &str) -> anyhow::Result<()> {
    println!("\n[fit] {group} — slope over the swept parameter");
    let xs_f: Vec<f64> = xs.iter().map(|&x| x as f64).collect();
    let mut ys = Vec::with_capacity(xs.len());
    for &x in xs {
        let ns = read_mean_ns(group, x)?;
        println!("  x={x:<7} mean ns = {ns:.2}");
        ys.push(ns);
    }
    let fit = fit_linear(&xs_f, &ys)?;
    if fit.per_byte < 0.0 {
        eprintln!(
            "warn: negative slope ({:.4} ns) — no marginal signal; the suggested constant \
             floors to 1 gas and is NOT meaningful.",
            fit.per_byte
        );
    }

    println!("\n=== fit: ns = fixed + slope * x   (1 gas = 0.01 ns) ===");
    println!(
        "  slope             = {:.4} ns   (used for the constant)",
        fit.per_byte
    );
    println!("  fixed (intercept) = {:.3e} ns  (discarded)", fit.bias);
    println!("  R²                = {:.6}", fit.r_squared);

    println!("\n=== suggested constants.toml value ===");
    println!("  {slope_const} = [{}, 0]", ns_to_gas(fit.per_byte));
    Ok(())
}

/// Read a finished value-size sweep done at a FIXED write count `count_n`, fit
/// `commit_ns = fixed + (count_n * per_byte) * size`, and suggest the per-byte
/// constant. The fitted slope is `count_n * per_byte`, so we divide by `count_n`;
/// the once-per-commit fixed cost is the discarded intercept.
///
/// `already_priced_ns_per_byte` (the value hashing charged separately via the
/// hash constants — the write hashes the value to build its trie leaf) is netted
/// out, so only the residual storage I/O belongs in the per-byte constant.
pub fn report_perbyte_sweep(
    group: &str,
    sizes: &[u64],
    count_n: u64,
    already_priced_ns_per_byte: f64,
    per_byte_const: &str,
) -> anyhow::Result<()> {
    println!("\n[fit] {group} — per-byte write cost at {count_n} writes/commit");
    let sizes_f: Vec<f64> = sizes.iter().map(|&s| s as f64).collect();
    let mut commit_ns = Vec::with_capacity(sizes.len());
    for &size in sizes {
        let ns = read_mean_ns(group, size)?;
        println!("  size={size:<7} commit ns = {ns:.0}");
        commit_ns.push(ns);
    }
    let fit = fit_linear(&sizes_f, &commit_ns)?;
    let raw_per_byte = fit.per_byte / count_n as f64;
    let net_per_byte = raw_per_byte - already_priced_ns_per_byte;

    println!("\n=== fit: commit ns = fixed + (count * per_byte) * size   (1 gas = 0.01 ns) ===");
    println!("  raw per_byte       = {:.4} ns/byte", raw_per_byte);
    println!(
        "  - hash per_byte    = {:.4} ns/byte  (charged via PER_BYTE_HASH_UPDATE)",
        already_priced_ns_per_byte
    );
    println!(
        "  = storage per_byte = {:.4} ns/byte  (used for the constant)",
        net_per_byte
    );
    println!("  R²                 = {:.6}", fit.r_squared);

    println!("\n=== suggested constants.toml value ===");
    println!("  {per_byte_const} = [{}, 0]", ns_to_gas(net_per_byte));
    Ok(())
}
