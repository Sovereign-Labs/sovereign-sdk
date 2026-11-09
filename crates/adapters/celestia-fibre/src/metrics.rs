use std::io::Write;

use sov_metrics::Metric;

pub const DOWNLOAD: &str = "sov_celestia_fibre_adapter_download";
pub const ENCODE: &str = "sov_celestia_fibre_adapter_encode";
pub const SUBMIT_PAY_FOR_FIBRE: &str = "sov_celestia_fibre_adapter_submit_pay_for_fibre";

#[derive(Debug)]
pub struct FibreMeasurement {
    pub name: &'static str,
    pub bytes: usize,
    pub elapsed: std::time::Duration,
    /// `None` for operations that only record successful runs.
    pub success: Option<bool>,
}

impl Metric for FibreMeasurement {
    fn measurement_name(&self) -> &'static str {
        self.name
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} bytes={},elapsed_us={}",
            self.name,
            self.bytes,
            self.elapsed.as_micros()
        )?;
        if let Some(success) = self.success {
            write!(buffer, ",success={}", success as u8)?;
        }
        Ok(())
    }
}
