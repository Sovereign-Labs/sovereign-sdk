//! Defines utilities for collecting runtime metrics from inside a SP1 VM
use sp1_sdk::HookEnv;

/// A custom callback for extracting metrics from the SP1 zkvm.
///
/// When the "bench" feature is enabled, this callback is registered as a syscall
/// in the SP1 VM and invoked whenever a function annotated with the `cycle_tracker`
/// macro is invoked.
pub fn metrics_hook(_env: HookEnv, buf: &[u8]) -> Vec<Vec<u8>> {
    let _ = sov_metrics::cycle_utils::deserialize_metrics_call(buf).unwrap();

    vec![]
}
