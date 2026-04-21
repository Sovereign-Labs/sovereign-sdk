use std::io::Write;

use sov_metrics::{write_escaped_field_value, Metric};
use sov_modules_api::DaSpec;

use crate::in_flight_blob::InFlightBlobInfo;

impl<Da: DaSpec> Metric for InFlightBlobInfo<Da> {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_in_flight_blobs_snapshot"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let last_known_state_json = serde_json::to_string(&self.last_known_state).unwrap();
        write!(
            buffer,
            "{} blob_iid=\"{}\",is_batch={},size_in_bytes={}i,was_resurrected={},duration_ms={}i,last_known_state=\"",
            self.measurement_name(),
            uuid::Uuid::from_u128(self.blob_iid).as_simple(),
            self.is_batch,
            self.size_in_bytes,
            self.was_resurrected,
            self.start_time.elapsed().as_millis(),
        )?;
        write_escaped_field_value(buffer, &last_known_state_json)?;
        buffer.write_all(b"\"")
    }
}

/// Gauge of the current total of in-flight blobs (blobs handed to the sender but not yet
/// finalized on the DA). Emitted as `sov_rollup_num_of_in_flight_blobs`.
///
/// Growing unboundedly indicates the DA submission pipeline cannot keep up with blob
/// production; correlate with `sov_rollup_in_flight_blobs_snapshot` to see per-blob state.
#[derive(Debug)]
struct InFlightBlobCountMetric {
    /// Number of blobs currently in-flight.
    count: u64,
}

impl Metric for InFlightBlobCountMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_num_of_in_flight_blobs"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} num_of_in_flight_blobs={}i",
            self.measurement_name(),
            self.count,
        )
    }
}

pub(super) fn track_num_of_in_flight_blobs(count: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(InFlightBlobCountMetric { count });
    });
}

/// InfluxDB line protocol requires at least one field per point; markers have no payload
/// of their own, so we emit a constant marker field.
const MARKER_FIELD: &str = "marker=1i";

#[derive(Debug)]
struct BlobScopeMarker {
    measurement_name: &'static str,
}

impl Metric for BlobScopeMarker {
    fn measurement_name(&self) -> &'static str {
        self.measurement_name
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{} {MARKER_FIELD}", self.measurement_name())
    }
}

pub(super) fn submit_blobs_enter_scope_marker(tracker: &sov_metrics::MetricsTracker) {
    tracker.submit(BlobScopeMarker {
        measurement_name: "sov_rollup_blobs_enter_scope",
    });
}

pub(super) fn submit_blobs_exit_scope_marker(tracker: &sov_metrics::MetricsTracker) {
    tracker.submit(BlobScopeMarker {
        measurement_name: "sov_rollup_blobs_exit_scope",
    });
}
