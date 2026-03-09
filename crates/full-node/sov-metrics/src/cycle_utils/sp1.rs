#[cfg(feature = "sp1")]
pub use actual_impl::*;
#[cfg(feature = "sp1")]
mod actual_impl {
    use crate::cycle_utils::{CycleMetric, MemoryInfo};

    /// File descriptor for the metrics hook, which is used to collect cycle duration data for functions.
    /// Can be any number, as long as it doesn't conflict with default/other hooks.
    pub const FD_METRICS_HOOK: u32 = 1001;

    /// Report the cycle count to the host, if available. Otherwise, this is a no-op.
    pub fn report_cycle_count(metric: CycleMetric) {
        sp1_lib::io::write(FD_METRICS_HOOK, &bincode::serialize(&metric).unwrap());
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
