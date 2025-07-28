#[cfg(feature = "sp1")]
pub use actual_impl::*;
#[cfg(feature = "sp1")]
mod actual_impl {
    use crate::cycle_utils::{CycleMetric, MemoryInfo};

    /// File descriptor for the cycle count hook, which is used to get the cycle count.
    /// Can be any number, as long as it doesn't conflict with default/other hooks.
    pub const FD_CYCLE_COUNT_HOOK: u32 = 1000;
    /// File descriptor for the metrics hook, which is used to collect cycle duration data for functions.
    /// Can be any number, as long as it doesn't conflict with default/other hooks.
    pub const FD_METRICS_HOOK: u32 = 1001;

    /// Report the cycle count to the host, if available. Otherwise, this is a no-op.
    pub fn report_cycle_count(metric: CycleMetric) {
        sp1_lib::io::write(FD_METRICS_HOOK, &bincode::serialize(&metric).unwrap());
    }

    /// Get the current cycle count of the sp1 zkvm, if available. Otherwise, return 0.
    pub fn get_cycle_count() -> u64 {
        // Writing zero bytes is a no-op, so we write &[0].
        sp1_lib::io::write(FD_CYCLE_COUNT_HOOK, &[0]);
        u64::from_le_bytes(
            sp1_lib::io::read_vec()
                .try_into()
                .expect("Failed to read cycle count before hook."),
        )
    }

    /// Returns how many bytes of heap are still available
    pub fn get_available_heap() -> MemoryInfo {
        MemoryInfo {
            free: 0x0C00_0000,
            used: 0,
        }
    }
}

#[cfg(not(feature = "sp1"))]
pub use facade::*;

#[cfg(not(feature = "sp1"))]
mod facade {
    use crate::cycle_utils::{CycleMetric, MemoryInfo};

    /// Get the current cycle count of the sp1 zkvm, if available. Otherwise, return 0.
    pub fn get_cycle_count() -> u64 {
        0
    }

    /// Report the cycle count to the host.
    pub fn report_cycle_count(_metric: CycleMetric) {
        panic!("Reporting sp1 cycle count without sp1 feature enabled");
    }

    /// Returns how many bytes of heap are still available
    pub fn get_available_heap() -> MemoryInfo {
        MemoryInfo {
            free: 0x0C00_0000,
            used: 0,
        }
    }
}
