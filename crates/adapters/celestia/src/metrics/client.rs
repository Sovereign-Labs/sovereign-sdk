//! Individual Celestia API calls.
//! Measure response time, is_success, and sometimes input params
//!  * header.NetworkHead
//!  * header.GetByHeight
//!  * share.GetNamespaceData
//!  * state.SubmitPayForBlob

use crate::metrics::RollupNamespace;
use sov_metrics::Metric;
use std::io::Write;
use std::marker::PhantomData;

/// Trait to define measurement names for different API calls
pub(crate) trait ApiCall: std::fmt::Debug + Sync + Send {
    fn measurement_name() -> &'static str;
}

// Marker types for each measurement
#[derive(Debug)]
pub(crate) struct HeaderGetByHeight;
#[derive(Debug)]
pub(crate) struct HeaderNetworkHead;
#[derive(Debug)]
pub(crate) struct ShareGetNamespaceData;
#[derive(Debug)]
pub(crate) struct StateSubmitPayForBlob;
#[derive(Debug)]
pub(crate) struct BlobGetAll;
#[derive(Debug)]
pub(crate) struct StateBalanceForAddress;
#[derive(Debug)]
pub(crate) struct HeaderSyncState;
#[derive(Debug)]
pub(crate) struct StateEstimateGasPrice;

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
pub(crate) struct MeasuredApiCall<T: ApiCall> {
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
pub(crate) struct MeasuredApiCallWithNamespace<T: ApiCall> {
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
pub(crate) type GetBlockHeaderMeasurement = MeasuredApiCall<HeaderGetByHeight>;
pub(crate) type GetChainHeadMeasurement = MeasuredApiCall<HeaderNetworkHead>;
pub(crate) type GetNamespaceDataMeasurement = MeasuredApiCallWithNamespace<ShareGetNamespaceData>;
pub(crate) type SubmitPayForBlob = MeasuredApiCallWithNamespace<StateSubmitPayForBlob>;
pub(crate) type BlobGetAllMeasurement = MeasuredApiCall<BlobGetAll>;
pub(crate) type StateBalanceForAddressMeasurement = MeasuredApiCall<StateBalanceForAddress>;
pub(crate) type HeaderSyncStateMeasurement = MeasuredApiCall<HeaderSyncState>;
pub(crate) type StateEstimateGasPriceMeasurement = MeasuredApiCall<StateEstimateGasPrice>;
