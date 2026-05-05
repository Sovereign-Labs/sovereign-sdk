use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};

use tokio::task_local;

use crate::influxdb::write_metadata_fields_for_telegraf;
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
    /// Name of the caller site, usually a function or method. Emitted as the `name` tag.
    pub name: String,
    /// The gas constant being tracked. Emitted as the `constant` tag.
    pub constant: String,
    /// Number of invocations of the gas constant within the caller site.
    pub num_invocations: i64,
    /// Arbitrary key/value metadata captured from the caller's arguments.
    /// Emitted as string **fields** (not tags) so unbounded values like hashes or heights
    /// do not cause series-cardinality explosion in InfluxDB.
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
        // `name` and `constant` are the only tags (both bounded — one value per annotated
        // function and per declared gas constant). `metadata` is emitted as string *fields*
        // rather than tags because callers pass unbounded values (hashes, heights, tx ids)
        // and using those as tags would explode InfluxDB series cardinality.
        write!(
            buffer,
            "{},name={},constant={} num_invocations={}",
            self.measurement_name(),
            self.name,
            self.constant,
            self.num_invocations,
        )?;
        write_metadata_fields_for_telegraf(buffer, &self.metadata)
    }
}
