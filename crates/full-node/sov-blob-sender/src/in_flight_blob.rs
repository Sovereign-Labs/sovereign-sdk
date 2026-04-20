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
