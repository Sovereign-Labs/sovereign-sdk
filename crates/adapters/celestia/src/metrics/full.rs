//! High level metrics of CelestiaService.
//! Can include multiple API calls.
//! Always measured on success.
use crate::metrics::RollupNamespace;
use celestia_types::row_namespace_data::NamespaceData;
use sov_metrics::Metric;
use std::io::Write;

/// High level metrics about fetching full block data: header and data.
#[derive(Debug)]
pub(crate) struct GetBlockMeasurement {
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
pub(crate) struct NamespaceDataMetrics {
    rows: usize,
    shares: usize,
}

impl NamespaceDataMetrics {
    pub fn new(data: &NamespaceData) -> Self {
        let rows = data.rows.len();
        let shares = data.rows.iter().map(|r| r.shares.len()).sum();
        Self { rows, shares }
    }
}

#[derive(Debug)]
pub(crate) struct BlobSubmitMeasurement {
    pub namespace: RollupNamespace,
    pub bytes: usize,
    pub lock_acquisition_time: std::time::Duration,
    pub submit_time: std::time::Duration,
    pub total_time: std::time::Duration,
}

impl Metric for BlobSubmitMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_submit_blob"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let namespace = self.namespace;
        let bytes = self.bytes;
        let lock_acquisition_us = self.lock_acquisition_time.as_micros();
        let submit_time_us = self.submit_time.as_micros();
        let total_time_us = self.total_time.as_micros();
        write!(
            buffer,
            "{name},namespace={namespace} bytes={bytes},lock_acquisition_us={lock_acquisition_us},submit_time_us={submit_time_us},total_time_us={total_time_us}"
        )
    }
}


#[derive(Debug)]
pub struct CelestiaAdapterStateMeasurement {
    pub balance: u64,
    pub gas_price: f64,
    pub sync_distance: u64,
}

impl Metric for CelestiaAdapterStateMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_periodic_data"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        Ok(())
    }
}