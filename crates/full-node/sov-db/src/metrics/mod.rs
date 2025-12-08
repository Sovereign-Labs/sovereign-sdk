use std::io::Write;

use rockbound::{SchemaKey, SchemaValue};
use sov_metrics::Metric;

use crate::schema::types::slot_key::{SlotKey, SlotValue};

pub mod nomt;

#[derive(Debug)]
pub struct StateMaterializationMetrics {
    /// How many key-value items have been materialized for user space
    pub user_items: usize,
    /// How many key-value items have been materialized for kernel space.
    pub kernel_items: usize,
    /// Cumulative size of keys across both namespaces.
    pub cumulative_keys_size: usize,
    /// Cumulative size of values across both namespaces.
    pub cumulative_values_size: usize,
    /// Max key size across all namespaces.
    pub max_key_size: usize,
    /// Max value size across all namespaces.
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
        "state_db_materialization"
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
