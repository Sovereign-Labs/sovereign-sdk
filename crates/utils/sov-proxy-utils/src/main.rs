use sov_metrics::init_metrics_tracker;
use sov_metrics::track_metrics;
use sov_metrics::MonitoringConfig;
use sov_proxy_utils::RootHashCheckMetric;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (_metrics_shutdown_sender, mut metrics_shutdown_receiver) = tokio::sync::watch::channel(());
    metrics_shutdown_receiver.mark_unchanged();
    init_metrics_tracker(
        &MonitoringConfig::standard(),
        metrics_shutdown_receiver.clone(),
    );

    let mut slot_number = 0_u64;
    for i in 0.. {
        let unique_state_roots = if i % 7 == 0 { 50 } else { 40 };

        //let consistency = check.check_consistency();
        track_metrics(|tracker| {
            tracker.submit(RootHashCheckMetric {
                slot_number,
                nodes_ok: slot_number,
                nodes_failed: 0,
                unique_state_roots,
            });
        });

        println!("Emit metrics {unique_state_roots}");

        slot_number = slot_number.saturating_add(1);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
