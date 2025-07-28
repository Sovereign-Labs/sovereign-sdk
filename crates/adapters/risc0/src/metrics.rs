//! Defines utilities for collecting runtime metrics from inside a Risc0 VM
use risc0_zkvm::Bytes;
/// The name of the syscall we use to collect metrics from the Risc0 VM.
pub use sov_metrics::cycle_utils::risc0::SYSCALL_NAME_METRICS;
use sov_metrics::cycle_utils::CycleMetric;

/// A custom callback for extracting metrics from the Risc0 zkvm.
///
/// When the "bench" feature is enabled, this callback is registered as a syscall
/// in the Risc0 VM and invoked whenever a function annotated with the `cycle_tracker`
/// macro is invoked.
pub fn metrics_callback(input: Bytes) -> anyhow::Result<Bytes> {
    let CycleMetric {
        name: metric,
        metadata,
        count: cycles_count,
        memory,
    } = sov_metrics::cycle_utils::deserialize_metrics_call(input.as_ref())?;

    sov_metrics::track_metrics(|tracker| {
        tracker.submit(sov_metrics::ZkVmExecutionChunk {
            name: metric,
            metadata,
            cycles_count,
            free_heap_bytes: memory.free as u64,
            memory_used: memory.used as u64,
        });
    });
    Ok(Bytes::new())
}
