use celestia_types::row_namespace_data::NamespaceData;
use celestia_types::state::RawTxResponse;
use sov_metrics::Metric;
use std::fmt::Formatter;
use std::io::Write;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RollupNamespace {
    Batch,
    Proof,
}

impl std::fmt::Display for RollupNamespace {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RollupNamespace::Batch => {
                write!(f, "batch")
            }
            RollupNamespace::Proof => {
                write!(f, "proof")
            }
        }
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
pub(crate) struct GetBlockMeasurement {
    pub height: u64,
    pub square_width: u16,
    pub fetch_header_time: std::time::Duration,
    // This includes both batch and proof rows, running concurrently.
    pub fetch_rows_time: std::time::Duration,
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
        // height is tag, the rest is field
        write!(
            buffer,
            "{},height={} square_width={},fetch_header_us={},fetch_rows_us={},build_data_us={},total_time_us={},batch_rows={},batch_shares={},proof_rows={},proof_shares={}",
            self.measurement_name(),
            self.height,
            self.square_width,
            self.fetch_header_time.as_micros(),
            self.fetch_rows_time.as_micros(),
            self.build_relevant_data.as_micros(),
            self.total_time.as_micros(),
            self.batch_ns_metrics.rows,
            self.batch_ns_metrics.shares,
            self.proof_ns_metrics.rows,
            self.proof_ns_metrics.shares,
        )
    }
}

#[derive(Debug)]
pub struct SuccessfulSubmitMeasurement {
    pub da_height: i64,
    pub gas_used: i64,
    pub response_code: u32,
}

#[derive(Debug)]
pub(crate) struct BlobSubmitMeasurement {
    pub namespace: RollupNamespace,
    pub bytes: usize,
    pub success_metrics: Option<SuccessfulSubmitMeasurement>,
    pub lock_acquisition_time: std::time::Duration,
    pub submit_time: std::time::Duration,
    pub total_time: std::time::Duration,
}

impl BlobSubmitMeasurement {
    pub fn new(
        namespace: RollupNamespace,
        result: &Result<RawTxResponse, jsonrpsee::core::ClientError>,
        bytes: usize,
        lock_acquisition_time: std::time::Duration,
        submit_time: std::time::Duration,
        total_time: std::time::Duration,
    ) -> Self {
        let success_metrics = match result {
            Ok(r) => Some(SuccessfulSubmitMeasurement {
                da_height: r.height,
                gas_used: r.gas_used,
                response_code: r.code,
            }),
            Err(_err) => None,
        };

        Self {
            namespace,
            bytes,
            success_metrics,
            lock_acquisition_time,
            submit_time,
            total_time,
        }
    }
}

impl Metric for BlobSubmitMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_submit_blob"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // success and namespace are tags, the rest are fields
        write!(
            buffer,
            "{},is_success={},namespace={} bytes={},lock_acquisition_us={},submit_time_us={},total_time_us={}",
            self.measurement_name(),
            self.success_metrics.is_some() as u8,
            self.namespace,
            self.bytes,
            self.lock_acquisition_time.as_micros(),
            self.submit_time.as_micros(),
            self.total_time.as_micros(),
        )?;
        if let Some(success_metrics) = &self.success_metrics {
            write!(
                buffer,
                ",status_code={},height={},gas_used={}",
                success_metrics.response_code, success_metrics.da_height, success_metrics.gas_used,
            )?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct GetBlockHeaderMeasurement {
    pub height: u64,
    pub fetch_header_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for GetBlockHeaderMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_get_header_at"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},height={},is_success={} total_time_us={}",
            self.measurement_name(),
            self.height,
            self.is_success as u8,
            self.fetch_header_time.as_micros(),
        )
    }
}

#[derive(Debug)]
pub(crate) struct GetChainHeadMeasurement {
    pub fetch_header_time: std::time::Duration,
    pub is_success: bool,
}

impl Metric for GetChainHeadMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_get_head_block"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},is_success={} total_time_us={}",
            self.measurement_name(),
            self.is_success as u8,
            self.fetch_header_time.as_micros(),
        )
    }
}
