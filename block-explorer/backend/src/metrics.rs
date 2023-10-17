use axum::extract::State;
use axum::routing::get;
use axum::Router;
use prometheus::{Encoder, TextEncoder};

pub fn router(metrics: Metrics) -> Router {
    Router::new()
        .route("/", get(metrics_handler))
        .with_state(metrics)
}

#[derive(Debug, Clone)]
pub struct Metrics {
    // TODO
}

async fn metrics_handler(State(_metrics): State<Metrics>) -> Vec<u8> {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];

    // Gather the metrics.
    let metric_families = prometheus::gather();
    // Encode them to send.
    encoder.encode(&metric_families, &mut buffer).unwrap();

    buffer
}
