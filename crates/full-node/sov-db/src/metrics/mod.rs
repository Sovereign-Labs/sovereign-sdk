use std::io::Write;

use sov_metrics::Metric;

use crate::schema::types::slot_key::{SlotKey, SlotValue};

pub mod nomt;

/// Shape of the state writes performed during one slot's materialization, split into
/// user-space (module state) and kernel-space (kernel state). Emitted as `sov_state_db_materialization`.
///
/// **What healthy looks like:** all fields scale roughly with transaction volume; `max_*` sizes
/// are stable across slots.
///
/// **Diagnostic signals:**
/// - `max_value_size` suddenly jumping → a module is writing a large blob to state (potential
///   unbounded-growth bug; find the module via slot replay).
/// - `cumulative_values_size` climbing without matching transaction volume → state bloat.
/// - `kernel_items` spiking while user workload is flat → kernel-level anomaly worth tracing.
///
/// **Correlate with:** `sov_rollup_slot_execution_time_us` (large materializations slow slot
/// processing) and `sov_nomt_commit_detailed` (downstream commit cost).
#[derive(Debug)]
pub struct StateMaterializationMetrics {
    pub user_items: usize,
    pub kernel_items: usize,
    pub cumulative_keys_size: usize,
    pub cumulative_values_size: usize,
    pub max_key_size: usize,
    pub max_value_size: usize,
}

impl StateMaterializationMetrics {
    pub(crate) fn new() -> Self {
        Self {
            user_items: 0,
            kernel_items: 0,
            cumulative_keys_size: 0,
            cumulative_values_size: 0,
            max_key_size: 0,
            max_value_size: 0,
        }
    }

    pub(crate) fn inc_user_items(&mut self) {
        self.user_items += 1;
    }

    pub(crate) fn inc_kernel_items(&mut self) {
        self.kernel_items += 1;
    }

    pub(crate) fn track_key_value_size(&mut self, key: &SlotKey, value: &Option<SlotValue>) {
        self.cumulative_keys_size += key.len();
        if let Some(value) = value {
            self.cumulative_values_size += value.as_ref().len();
        }
        self.max_key_size = std::cmp::max(self.max_key_size, key.len());
        if let Some(value) = value {
            self.max_value_size = std::cmp::max(self.max_value_size, value.as_ref().len());
        }
    }
}

impl Metric for StateMaterializationMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_state_db_materialization"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} user_items={},kernel_items={},c_key_size={},c_value_size={},max_key_size={},max_value_size={}",
            self.measurement_name(),
            self.user_items,
            self.kernel_items,
            self.cumulative_keys_size,
            self.cumulative_values_size,
            self.max_key_size,
            self.max_value_size,
        )
    }
}
