use sov_metrics::Metric;
use std::io::Write;

#[derive(Debug)]
enum MetricContext {
    CpuHavey,
    StateHavey,
    ManyValues,
}

#[derive(Debug)]
struct SyntheticLoadMetric {
    duration: std::time::Duration,
    context: MetricContext,
}

impl Metric for SyntheticLoadMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_value_setter"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},context={:?} duration_us={}",
            self.measurement_name(),
            self.context,
            self.duration.as_micros()
        )
    }
}

pub(crate) fn submit_cpu_heavy_metric(start_time: std::time::Instant) {
    let duration = start_time.elapsed();
    sov_metrics::track_metrics(|tracker| {
        let metric = SyntheticLoadMetric {
            duration,
            context: MetricContext::CpuHavey,
        };
        tracker.submit(metric);
    });
}

pub(crate) fn submit_state_heavy_metric(start_time: std::time::Instant) {
    let duration = start_time.elapsed();
    sov_metrics::track_metrics(|tracker| {
        let metric = SyntheticLoadMetric {
            duration,
            context: MetricContext::StateHavey,
        };
        tracker.submit(metric);
    });
}

pub(crate) fn submit_many_values_metric(start_time: std::time::Instant) {
    let duration = start_time.elapsed();
    sov_metrics::track_metrics(|tracker| {
        let metric = SyntheticLoadMetric {
            duration,
            context: MetricContext::ManyValues,
        };
        tracker.submit(metric);
    });
}
