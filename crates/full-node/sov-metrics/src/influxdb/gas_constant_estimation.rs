use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};

use tokio::task_local;

use crate::influxdb::safe_telegraf_string;
use crate::{timestamp, Metric, MetricsTracker};

task_local! {
    /// A map of gas constants and their associated weight.
    pub static GAS_CONSTANTS: RefCell<GasConstantTracker>;
}

/// A structure used to track the usage of gas constants.
#[derive(Debug, Clone, Default, derive_more::Deref, derive_more::DerefMut)]
pub struct GasConstantTracker(HashMap<String, i64>);

impl GasConstantTracker {
    /// Returns the difference between the current and the previous gas constant usage.
    /// Consumes both trackers.
    pub fn diff(mut self, previous: Self) -> Self {
        for (constant, weight) in previous.0.into_iter() {
            if let Some(current_weight) = self.0.get(&constant) {
                if *current_weight != weight {
                    self.0.insert(constant, *current_weight - weight);
                } else {
                    self.0.remove(&constant);
                }
            }
        }

        self
    }

    /// Emits the gas constant usage as telegraf metrics.
    pub fn report_gas_constants_usage(
        &self,
        method_name: &str,
        tagged_inputs: Vec<(String, String)>,
    ) {
        for (constant, weight) in self.0.iter() {
            crate::track_metrics(|tracker| {
                let point = GasConstantMetric {
                    name: method_name.to_string(),
                    constant: constant.to_string(),
                    num_invocations: *weight,
                    metadata: tagged_inputs.clone(),
                };
                tracker.track_gas_constants_usage(point);
            });
        }
    }
}

#[derive(Debug)]
pub struct GasConstantMetric {
    /// Name of the caller site, usually a function or method
    pub name: String,
    /// The gas constant tracked
    pub constant: String,
    /// A numerical value representing the number of invocations of the gas constant
    pub num_invocations: i64,
    /// Additional metadata to be included in the metrics. The metadata is added as a
    /// measurement attribute according to the [influxdb line protocol](https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/)
    /// We are parsing the metadata in the `tag_key=tag_value` format of influxdb.
    /// This can be used to filter metrics data in telegraf, by querying metrics for some
    /// specific metadata.
    pub metadata: Vec<(String, String)>,
}

impl MetricsTracker {
    /// Tracks HTTP-related metrics.
    fn track_gas_constants_usage(&self, point: GasConstantMetric) {
        let timestamp = timestamp();
        self.submit_with_time(timestamp, point);
    }
}

impl Metric for GasConstantMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_gas_constant"
    }

    fn write_to_csv(&self, writer: &mut super::csv_helper::CsvWriters) -> io::Result<()> {
        let writer = &mut writer.constant_writer;

        let meta = &self.metadata;
        let maybe_pre_state_root = meta.iter().find(|(k, _)| k == "pre_state_root");
        if let Some(pre_state_root) = maybe_pre_state_root {
            let row = format!(
                "{},{},{},{}\n",
                self.name, self.constant, self.num_invocations, pre_state_root.1
            );
            writer.write_all(row.as_bytes())?;
            writer.flush()?;
        }
        Ok(())
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},name={},constant={}",
            self.measurement_name(),
            self.name,
            self.constant,
        )?;

        let parsed_metadata = self
            .metadata
            .iter()
            .map(|(key, value)| {
                // Replace spaces with underscores to make them compatible with telegraf
                // Source: (Special telegraf characters)[`https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/#special-characters`]
                let telegraf_formatted_key = safe_telegraf_string(key);

                format!("{telegraf_formatted_key}={value}")
            })
            .collect::<Vec<_>>();

        if !parsed_metadata.is_empty() {
            // We are adding the metadata as measurement tags in the influxdb line protocol.
            write!(buffer, ",{}", parsed_metadata.join(","))?;
        }

        // Now actual value. Note, leading space is important.
        write!(buffer, " num_invocations={}", self.num_invocations)?;
        Ok(())
    }
}
