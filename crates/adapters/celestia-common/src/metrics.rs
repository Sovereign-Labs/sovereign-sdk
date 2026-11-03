#[derive(Debug, Clone, Copy)]
pub enum RollupNamespace {
    Batch,
    Proof,
}

impl std::fmt::Display for RollupNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RollupNamespace::Batch => write!(f, "batch"),
            RollupNamespace::Proof => write!(f, "proof"),
        }
    }
}

use celestia_types::namespace_data::NamespaceData;
use sov_metrics::Metric;
use std::io::Write;
use std::marker::PhantomData;

/// Trait to define measurement names for different API calls
pub trait ApiCall: std::fmt::Debug + Sync + Send {
    fn measurement_name() -> &'static str;
}

// Marker types for each measurement
#[derive(Debug)]
pub struct HeaderGetByHeight;
#[derive(Debug)]
pub struct HeaderNetworkHead;
#[derive(Debug)]
pub struct ShareGetNamespaceData;
#[derive(Debug)]
pub struct StateSubmitPayForBlob;
#[derive(Debug)]
pub struct BlobGetAll;
#[derive(Debug)]
pub struct StateBalanceForAddress;
#[derive(Debug)]
pub struct HeaderSyncState;
#[derive(Debug)]
pub struct StateEstimateGasPrice;

impl ApiCall for HeaderGetByHeight {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_header_get_by_height"
    }
}

impl ApiCall for HeaderNetworkHead {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_header_network_head"
    }
}

impl ApiCall for ShareGetNamespaceData {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_share_get_namespace_data"
    }
}

impl ApiCall for StateSubmitPayForBlob {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_state_submit_pay_for_blob"
    }
}

impl ApiCall for BlobGetAll {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_blob_get_all"
    }
}

impl ApiCall for StateBalanceForAddress {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_state_balance_for_address"
    }
}

impl ApiCall for HeaderSyncState {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_header_sync_state"
    }
}

impl ApiCall for StateEstimateGasPrice {
    fn measurement_name() -> &'static str {
        "sov_celestia_adapter_state_estimate_gas_price"
    }
}

/// Generic measurement for simple API calls without additional parameters
#[derive(Debug)]
pub struct MeasuredApiCall<T: ApiCall> {
    pub response_time: std::time::Duration,
    pub is_success: bool,
    _phantom: PhantomData<T>,
}

impl<T: ApiCall> MeasuredApiCall<T> {
    pub fn new(response_time: std::time::Duration, is_success: bool) -> Self {
        Self {
            response_time,
            is_success,
            _phantom: PhantomData,
        }
    }
}

impl<T: ApiCall> Metric for MeasuredApiCall<T> {
    fn measurement_name(&self) -> &'static str {
        T::measurement_name()
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let response_time_us = self.response_time.as_micros();
        let is_success = self.is_success as u8;
        write!(
            buffer,
            "{name},is_success={is_success} response_time_us={response_time_us}"
        )
    }
}

/// Generic measurement for API calls that include a namespace parameter
#[derive(Debug)]
pub struct MeasuredApiCallWithNamespace<T: ApiCall> {
    pub namespace: RollupNamespace,
    pub response_time: std::time::Duration,
    pub is_success: bool,
    _phantom: PhantomData<T>,
}

impl<T: ApiCall> MeasuredApiCallWithNamespace<T> {
    pub fn new(
        namespace: RollupNamespace,
        response_time: std::time::Duration,
        is_success: bool,
    ) -> Self {
        Self {
            namespace,
            response_time,
            is_success,
            _phantom: PhantomData,
        }
    }
}

impl<T: ApiCall> Metric for MeasuredApiCallWithNamespace<T> {
    fn measurement_name(&self) -> &'static str {
        T::measurement_name()
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let namespace = self.namespace;
        let response_time_us = self.response_time.as_micros();
        let is_success = self.is_success as u8;
        write!(
            buffer,
            "{name},is_success={is_success},namespace={namespace} response_time_us={response_time_us}"
        )
    }
}

// Type aliases for concrete types
pub type GetBlockHeaderMeasurement = MeasuredApiCall<HeaderGetByHeight>;
pub type GetChainHeadMeasurement = MeasuredApiCall<HeaderNetworkHead>;
pub type GetNamespaceDataMeasurement = MeasuredApiCallWithNamespace<ShareGetNamespaceData>;
pub type SubmitPayForBlob = MeasuredApiCallWithNamespace<StateSubmitPayForBlob>;
pub type BlobGetAllMeasurement = MeasuredApiCall<BlobGetAll>;
pub type StateBalanceForAddressMeasurement = MeasuredApiCall<StateBalanceForAddress>;
pub type HeaderSyncStateMeasurement = MeasuredApiCall<HeaderSyncState>;
pub type StateEstimateGasPriceMeasurement = MeasuredApiCall<StateEstimateGasPrice>;

/// High level metrics about fetching full block data: header and data.
#[derive(Debug)]
pub struct GetBlockMeasurement {
    pub height: u64,
    pub square_width: u16,
    // This includes all futures running concurrently
    pub futures_time: std::time::Duration,
    pub build_relevant_data: std::time::Duration,
    pub batch_ns_metrics: NamespaceDataMetrics,
    pub proof_ns_metrics: NamespaceDataMetrics,
    pub total_time: std::time::Duration,
}

impl Metric for GetBlockMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_get_block"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let height = self.height;
        let square_width = self.square_width;
        let futures_us = self.futures_time.as_micros();
        let build_data_us = self.build_relevant_data.as_micros();
        let total_time_us = self.total_time.as_micros();
        let batch_rows = self.batch_ns_metrics.rows;
        let batch_shares = self.batch_ns_metrics.shares;
        let proof_rows = self.proof_ns_metrics.rows;
        let proof_shares = self.proof_ns_metrics.shares;

        write!(
            buffer,
            "{name} height={height},square_width={square_width},futures_us={futures_us},build_data_us={build_data_us},total_time_us={total_time_us},batch_rows={batch_rows},batch_shares={batch_shares},proof_rows={proof_rows},proof_shares={proof_shares}",
        )
    }
}

#[derive(Debug)]
pub struct NamespaceDataMetrics {
    rows: usize,
    shares: usize,
}

impl NamespaceDataMetrics {
    pub fn new(data: &NamespaceData) -> Self {
        let rows = data.rows();
        let shares = rows.iter().map(|r| r.shares.len()).sum();
        Self {
            rows: rows.len(),
            shares,
        }
    }
}
