pub mod cmd;
pub mod fit;
pub mod reports;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sp1_sdk::blocking::Elf;

pub const SP1_SDK_VERSION: &str = "6.1.0";

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

pub fn load_guest_elf(path: &str) -> anyhow::Result<Elf> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("guest ELF not found at {path}; did the build script run?"))?;
    if bytes.is_empty() {
        anyhow::bail!("guest ELF at {path} is empty");
    }
    Ok(Elf::from(bytes))
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
