//! Individual Celestia API calls.
//! Measure response time, is_success, and sometimes input params
//!  * header.NetworkHead
//!  * header.GetByHeight
//!  * share.GetNamespaceData
//!  * state.SubmitPayForBlob

use crate::metrics::RollupNamespace;
use sov_metrics::Metric;
use std::io::Write;

#[derive(Debug)]
pub(crate) struct GetBlockHeaderMeasurement {
    pub height: u64,
    pub response_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for GetBlockHeaderMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_header_get_by_height"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let height = self.height;
        let response_time_us = self.response_time.as_micros();
        let is_success = self.is_success as u8;
        write!(
            buffer,
            "{name} height={height},is_success={is_success},response_time_us={response_time_us}",
        )
    }
}

#[derive(Debug)]
pub(crate) struct GetChainHeadMeasurement {
    pub response_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for GetChainHeadMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_header_network_head"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let response_time_us = self.response_time.as_micros();
        let is_success = self.is_success as u8;
        write!(
            buffer,
            "{name} is_success={is_success},response_time_us={response_time_us}"
        )
    }
}

#[derive(Debug)]
pub(crate) struct GetNamespaceDataMeasurement {
    pub height: u64,
    pub namespace: RollupNamespace,
    pub response_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for GetNamespaceDataMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_share_get_namespace_data"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let height = self.height;
        let namespace = self.namespace;
        let response_time_us = self.response_time.as_micros();
        let is_success = self.is_success as u8;
        write!(buffer, "{name} is_success={is_success},namespace={namespace},height={height},response_time_us={response_time_us}")
    }
}

#[derive(Debug)]
pub(crate) struct SubmitPayForBlob {
    pub namespace: RollupNamespace,
    pub response_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for SubmitPayForBlob {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_state_submit_pay_for_blob"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let is_success = self.is_success as u8;
        let response_time_us = self.response_time.as_micros();
        let namespace = self.namespace;
        write!(buffer, "{name} is_success={is_success},namespace={namespace},response_time_us={response_time_us}")
    }
}
