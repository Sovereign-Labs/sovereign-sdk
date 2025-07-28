use serde::{Deserialize, Serialize};

/// Cycle utils for the risc0 zkvm
pub mod risc0;

/// Cycle utils for the sp1 zkvm
pub mod sp1;

/// Metric to report to the zkvm
#[derive(Debug, Serialize, Deserialize)]
pub struct CycleMetric {
    /// Identifier of the tagged function
    pub name: String,
    /// Metadata to include with the metric
    pub metadata: Vec<(String, String)>,
    /// Number of cycles
    pub count: u64,
    /// Memory usage information
    pub memory: MemoryInfo,
}

#[cfg(feature = "native")]
/// Deserialize the output of the metrics syscall
pub fn deserialize_metrics_call(serialized: &[u8]) -> anyhow::Result<CycleMetric> {
    Ok(bincode::deserialize::<CycleMetric>(serialized)?)
}

/// Information on memory consumption for the zkvm
#[derive(Serialize, Deserialize, Debug)]
pub struct MemoryInfo {
    /// Amount of bytes of memory still free to use.
    pub free: usize,
    /// Amount of bytes of memory used during the execution of the block (and not reclaimed after execution is complete).
    pub used: usize,
}
