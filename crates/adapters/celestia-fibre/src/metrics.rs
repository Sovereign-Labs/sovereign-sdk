use std::io::Write;
use std::time::Duration;

/// An operation of [`crate::CelestiaFibreDaService`] that reports a metric.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Operation {
    /// One attempt of fetching a header by height.
    GetBlockHeader,
    /// One attempt of fetching the network head.
    GetHeadBlockHeader,
    /// One attempt of fetching the namespace data of a block.
    GetNamespaceData,
    /// One attempt of broadcasting a `MsgPayForFibre`.
    SubmitPayForFibre,
    /// One attempt of querying the status of a submitted tx.
    TxStatus,
    /// One attempt of downloading a Fibre blob from validators.
    FibreDownload,
    /// One attempt of encoding a Fibre blob and uploading it to validators.
    FibreUpload,
}

impl Operation {
    fn measurement_name(self) -> &'static str {
        match self {
            Self::GetBlockHeader => "sov_celestia_fibre_adapter_get_block_header",
            Self::GetHeadBlockHeader => "sov_celestia_fibre_adapter_get_head_block_header",
            Self::GetNamespaceData => "sov_celestia_fibre_adapter_get_namespace_data",
            Self::SubmitPayForFibre => "sov_celestia_fibre_adapter_submit_pay_for_fibre",
            Self::TxStatus => "sov_celestia_fibre_adapter_tx_status",
            Self::FibreDownload => "sov_celestia_fibre_adapter_fibre_download",
            Self::FibreUpload => "sov_celestia_fibre_adapter_fibre_upload",
        }
    }
}

/// Duration and outcome of one attempt of an [`Operation`].
#[derive(Debug)]
pub(crate) struct OperationMeasurement {
    operation: Operation,
    response_time: Duration,
    is_success: bool,
    /// Blob size, for Fibre downloads and uploads.
    bytes: Option<usize>,
}

impl sov_metrics::Metric for OperationMeasurement {
    fn measurement_name(&self) -> &'static str {
        self.operation.measurement_name()
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},is_success={} response_time_us={}",
            self.measurement_name(),
            self.is_success as u8,
            self.response_time.as_micros()
        )?;
        if let Some(bytes) = self.bytes {
            write!(buffer, ",bytes={bytes}")?;
        }
        Ok(())
    }
}

/// Reports one attempt of `operation`. `bytes` is the blob size for Fibre downloads and uploads.
pub(crate) fn track(
    operation: Operation,
    response_time: Duration,
    is_success: bool,
    bytes: Option<usize>,
) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(OperationMeasurement {
            operation,
            response_time,
            is_success,
            bytes,
        });
    });
}

#[cfg(test)]
mod tests {
    use sov_metrics::Metric;

    use super::*;

    #[test]
    fn serializes_blob_size_when_present() {
        let measurement = OperationMeasurement {
            operation: Operation::FibreUpload,
            response_time: Duration::from_micros(0x1234),
            is_success: true,
            bytes: Some(0x5678),
        };
        let mut buffer = Vec::new();

        measurement.serialize_for_telegraf(&mut buffer).unwrap();

        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "sov_celestia_fibre_adapter_fibre_upload,is_success=1 response_time_us=4660,bytes=22136"
        );
    }
}
