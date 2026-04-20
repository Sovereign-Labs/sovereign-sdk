use std::io::Write;

use sov_metrics::Metric;
use sov_modules_api::DaSpec;
use tokio::task::JoinHandle;

use crate::{BlobExecutionStatus, BlobInternalId};

#[derive(Debug)]
pub struct InFlightBlob<Da: DaSpec> {
    pub info: InFlightBlobInfo<Da>,
    pub handle: JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub struct InFlightBlobInfo<Da: DaSpec> {
    pub blob_iid: BlobInternalId,
    pub start_time: std::time::Instant,
    pub is_batch: bool,
    pub size_in_bytes: u64,
    pub was_resurrected: bool,
    pub last_known_state: BlobExecutionStatus<Da>,
}

impl<Da: DaSpec> Metric for InFlightBlobInfo<Da> {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_in_flight_blobs_snapshot"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} blob_iid=\"{}\",is_batch={},size_in_bytes={}i,was_resurrected={},duration_ms={}i,last_known_state=\"{}\"",
            self.measurement_name(),
            uuid::Uuid::from_u128(self.blob_iid).as_simple(),
            self.is_batch,
            self.size_in_bytes,
            self.was_resurrected,
            self.start_time.elapsed().as_millis(),
            serde_json::to_string(&self.last_known_state).unwrap().replace("\\", "\\\\").replace("\"", "\\\""),
        )
    }
}

/// Gauge of the current total of in-flight blobs (blobs handed to the sender but not yet
/// finalized on the DA). Emitted as `sov_rollup_num_of_in_flight_blobs`.
///
/// Growing unboundedly indicates the DA submission pipeline cannot keep up with blob
/// production; correlate with `sov_rollup_in_flight_blobs_snapshot` to see per-blob state.
#[derive(Debug)]
pub struct InFlightBlobCountMetric {
    /// Number of blobs currently in-flight.
    pub count: u64,
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

pub fn track_num_of_in_flight_blobs(count: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(InFlightBlobCountMetric { count });
    });
}

/// Marker emitted immediately before a batch of `InFlightBlobInfo` snapshots.
/// Emitted as `sov_rollup_blobs_enter_scope`; use together with `BlobsExitScopeMarker`
/// to group snapshots belonging to a single reporting cycle.
#[derive(Debug)]
pub struct BlobsEnterScopeMarker;

impl Metric for BlobsEnterScopeMarker {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_blobs_enter_scope"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{} foo=1", self.measurement_name())
    }
}

/// Marker emitted immediately after a batch of `InFlightBlobInfo` snapshots.
/// Emitted as `sov_rollup_blobs_exit_scope`; see `BlobsEnterScopeMarker` for the paired event.
#[derive(Debug)]
pub struct BlobsExitScopeMarker;

impl Metric for BlobsExitScopeMarker {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_blobs_exit_scope"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{} foo=1", self.measurement_name())
    }
}
