//! Adapts SP1 prover-gas bench results to the shared OLS cost-model fit.
//!
//! The fit math lives in the `sov-gas-tools` crate (shared with the native
//! microbenches and downstream rollup benches); this module only extracts
//! `(input_size, per-call prover gas)` from [`BenchResult`] and delegates.

pub use sov_gas_tools::fit::LinearFit;

use crate::BenchResult;

/// Fit `prover_gas_per_call = bias + per_byte * input_size` over the bench results.
pub fn fit_prover_gas_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let input_sizes: Vec<f64> = results.iter().map(|r| r.input_size as f64).collect();
    let prover_gas: Vec<f64> = results.iter().map(|r| r.per_iter_prover_gas()).collect();
    sov_gas_tools::fit::fit_linear(&input_sizes, &prover_gas)
}
