// Host-only harness; the workspace `clippy::float_arithmetic` deny targets zkVM code, not this.
#![allow(clippy::float_arithmetic)]

pub mod cmd;
pub mod fit;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sp1_sdk::blocking::Elf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub input_size: u32,
    pub iterations: u32,
    pub prover_gas: u64,
    pub total_cycles: u64,
    pub region_cycles: u64,
}

impl BenchResult {
    pub fn per_iter_prover_gas(&self) -> f64 {
        self.prover_gas as f64 / self.iterations.max(1) as f64
    }

    pub fn per_iter_region_cycles(&self) -> f64 {
        self.region_cycles as f64 / self.iterations.max(1) as f64
    }
}

pub fn load_guest_elf(path: &str) -> anyhow::Result<Elf> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("guest ELF not found at {path}; did the build script run?"))?;
    if bytes.is_empty() {
        anyhow::bail!("guest ELF at {path} is empty");
    }
    Ok(Elf::from(bytes))
}

/// Rounds a cost for the single metered dimension, never flooring a positive cost to zero.
pub fn round_at_least_one(total: f64) -> u64 {
    let rounded = total.round() as u64;
    if total > 0.0 && rounded == 0 {
        1
    } else {
        rounded
    }
}
