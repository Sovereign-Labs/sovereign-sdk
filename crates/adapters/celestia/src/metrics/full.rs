//! High level metrics of CelestiaService.
//! Can include multiple API calls.
//! Always measured on success.
use crate::metrics::RollupNamespace;
use crate::verifier::address::CelestiaAddress;
use crate::CompressOnSubmit;
use sov_metrics::Metric;
use std::io::Write;

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

/// Batch-blob compression outcome at submission. Emitted for every batch submit
/// (including `mode=off`) so the compression ratio is observable as a baseline.
#[derive(Debug)]
pub(crate) struct BlobCompressionMeasurement {
    /// Emission mode tag: `off` or `lz4` (the configured policy).
    pub mode: CompressOnSubmit,
    /// Rollup (pre-encoding) payload length.
    pub rollup_bytes: usize,
    /// DA (post-encoding) payload length.
    pub da_bytes: usize,
}

impl BlobCompressionMeasurement {
    pub fn new(mode: CompressOnSubmit, rollup_bytes: usize, da_bytes: usize) -> Self {
        Self {
            mode,
            rollup_bytes,
            da_bytes,
        }
    }
}

impl Metric for BlobCompressionMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_compress_blob"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let mode = self.mode;
        let rollup_bytes = self.rollup_bytes;
        let da_bytes = self.da_bytes;
        let saved_bytes = rollup_bytes.saturating_sub(da_bytes);
        // da/rollup in basis points (10000 = no reduction). Integer-only.
        let ratio_bps = if rollup_bytes == 0 {
            10_000
        } else {
            (da_bytes as u64 * 10_000) / rollup_bytes as u64
        };
        write!(
            buffer,
            "{name},mode={mode} rollup_bytes={rollup_bytes},da_bytes={da_bytes},saved_bytes={saved_bytes},ratio_bps={ratio_bps}"
        )
    }
}

#[derive(Debug)]
pub struct CelestiaAdapterStateMeasurement {
    pub signer: CelestiaAddress,
    pub balance: u64,
    pub gas_price: f64,
    pub sync_distance: u64,
}

impl Metric for CelestiaAdapterStateMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_adapter_periodic_data"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let signer = &self.signer;
        let balance = self.balance;
        let gas_price = self.gas_price;
        let sync_distance = self.sync_distance;
        // Celestia minimum gas price can be 0.000001 utia, so 6 decimal places is sufficient.
        // See: https://forum.celestia.org/t/cip-price-enforcement/1351/5
        write!(
            buffer,
            "{name},signer={signer} balance={balance},gas_price={gas_price:.6},sync_distance={sync_distance}"
        )
    }
}
