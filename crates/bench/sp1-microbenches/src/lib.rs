#![allow(missing_docs)]

pub mod fit;
pub mod reports;

use serde::{Deserialize, Serialize};

/// Single bench data point: outcome of executing the guest with a given input shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub byte_len: u32,
    pub iterations: u32,
    pub prover_gas: u64,
    pub total_cycles: u64,
    pub hash_loop_cycles: u64,
    pub invocations: u64,
}

impl BenchResult {
    pub fn per_iter_prover_gas(&self) -> f64 {
        self.prover_gas as f64 / self.iterations.max(1) as f64
    }
    pub fn per_iter_hash_cycles(&self) -> f64 {
        self.hash_loop_cycles as f64 / self.iterations.max(1) as f64
    }
}
