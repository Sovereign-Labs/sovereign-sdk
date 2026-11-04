use std::io::Write;

use sov_metrics::Metric;

#[derive(Debug)]
pub struct FibreDownloadMeasurement {
    pub bytes: usize,
    pub elapsed: std::time::Duration,
    pub success: bool,
}

impl Metric for FibreDownloadMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_fibre_adapter_download"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} bytes={},elapsed_us={},success={}",
            self.measurement_name(),
            self.bytes,
            self.elapsed.as_micros(),
            self.success as u8
        )
    }
}

#[derive(Debug)]
pub struct FibreEncodeMeasurement {
    pub bytes: usize,
    pub elapsed: std::time::Duration,
}

impl Metric for FibreEncodeMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_fibre_adapter_encode"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} bytes={},elapsed_us={}",
            self.measurement_name(),
            self.bytes,
            self.elapsed.as_micros()
        )
    }
}

#[derive(Debug)]
pub struct SubmitPayForFibreMeasurement {
    pub bytes: usize,
    pub elapsed: std::time::Duration,
    pub success: bool,
}

impl Metric for SubmitPayForFibreMeasurement {
    fn measurement_name(&self) -> &'static str {
        "sov_celestia_fibre_adapter_submit_pay_for_fibre"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} bytes={},elapsed_us={},success={}",
            self.measurement_name(),
            self.bytes,
            self.elapsed.as_micros(),
            self.success as u8
        )
    }
}
