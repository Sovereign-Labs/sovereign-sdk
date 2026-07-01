// Host-only benchmark harness; the workspace `clippy::float_arithmetic` deny exists to prevent
// native/zkVM divergence, which doesn't apply here.
#![allow(clippy::float_arithmetic)]

pub mod cmd;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sp1_sdk::blocking::Elf;

pub use sov_gas_tools::fit::LinearFit;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub input_size: u32,
    pub iterations: u32,
    pub prover_gas: u64,
    pub total_cycles: u64,
    pub region_cycles: u64,
    pub invocations: u64,
}

impl BenchResult {
    pub fn per_iter_prover_gas(&self) -> f64 {
        self.prover_gas as f64 / self.iterations.max(1) as f64
    }

    pub fn per_iter_region_cycles(&self) -> f64 {
        self.region_cycles as f64 / self.iterations.max(1) as f64
    }
}

/// Fit `prover_gas_per_call = bias + per_byte * input_size` over the bench results.
pub fn fit_prover_gas_per_byte(results: &[BenchResult]) -> anyhow::Result<LinearFit> {
    let input_sizes: Vec<f64> = results.iter().map(|r| r.input_size as f64).collect();
    let prover_gas: Vec<f64> = results.iter().map(|r| r.per_iter_prover_gas()).collect();
    Ok(sov_gas_tools::fit::fit_linear(&input_sizes, &prover_gas)?)
}

pub fn load_guest_elf(path: &str) -> anyhow::Result<Elf> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("guest ELF not found at {path}; did the build script run?"))?;
    if bytes.is_empty() {
        anyhow::bail!("guest ELF at {path} is empty");
    }
    Ok(Elf::from(bytes))
}
