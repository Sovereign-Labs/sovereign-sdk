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

pub fn track_num_of_in_flight_blobs(count: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit_inline(
            "sov_rollup_num_of_in_flight_blobs",
            format!("num_of_in_flight_blobs={count}i"),
        );
    });
}
