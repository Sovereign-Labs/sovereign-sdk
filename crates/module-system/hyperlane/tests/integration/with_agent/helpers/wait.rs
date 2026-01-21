//! Utilities for waiting on relayer state using metrics.

use std::time::Duration;

use super::metrics::{MetricsError, RelayerMetricsClient};

/// Configuration for waiting on relayer metrics.
pub struct RelayerWaitConfig {
    /// Maximum time to wait before timing out.
    pub timeout: Duration,
    /// Interval between polling metrics.
    pub poll_interval: Duration,
    /// Maximum allowed increase in error count before failing.
    pub max_error_increase: u64,
}

impl Default for RelayerWaitConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(500),
            max_error_increase: 5,
        }
    }
}

/// Error type for wait operations.
#[derive(Debug)]
pub enum WaitError {
    /// Timed out waiting for the condition.
    Timeout(String),
    /// Relayer encountered too many errors.
    TooManyErrors { initial: u64, current: u64 },
    /// Relayer encountered a critical error.
    CriticalError,
    /// Failed to fetch metrics.
    MetricsError(MetricsError),
}

impl std::fmt::Display for WaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitError::Timeout(msg) => write!(f, "Timeout: {msg}"),
            WaitError::TooManyErrors { initial, current } => {
                write!(
                    f,
                    "Relayer error count increased from {initial} to {current}"
                )
            }
            WaitError::CriticalError => write!(f, "Relayer encountered a critical error"),
            WaitError::MetricsError(e) => write!(f, "Metrics error: {e}"),
        }
    }
}

impl std::error::Error for WaitError {}

impl From<MetricsError> for WaitError {
    fn from(e: MetricsError) -> Self {
        WaitError::MetricsError(e)
    }
}

/// Waits for the relayer to process messages from origin to remote (destination).
///
/// This polls the `hyperlane_messages_processed_count` metric until it reaches
/// the expected count or times out.
pub async fn wait_for_messages_processed(
    metrics: &RelayerMetricsClient,
    origin: &str,
    remote: &str,
    expected_count: u64,
    config: RelayerWaitConfig,
) -> Result<(), WaitError> {
    let start = std::time::Instant::now();
    let initial_errors = get_error_count_safe(metrics).await;

    tracing::info!(
        %origin,
        %remote,
        %expected_count,
        timeout_secs = ?config.timeout.as_secs(),
        "Waiting for messages to be processed"
    );

    // Debug: dump all available metrics once at start
    if let Ok(text) = metrics.fetch_metrics().await {
        eprintln!("[DEBUG] === Available metrics (non-comment lines) ===");
        for line in text.lines() {
            if !line.starts_with('#') && !line.is_empty() {
                eprintln!("[DEBUG] {line}");
            }
        }
        eprintln!("[DEBUG] === End of metrics ===");
    }

    loop {
        // Check for timeout
        if start.elapsed() > config.timeout {
            let current_count = metrics
                .get_messages_processed_count(origin, remote)
                .await
                .unwrap_or(0);
            return Err(WaitError::Timeout(format!(
                "Messages processed: {current_count}/{expected_count} after {:?}",
                start.elapsed()
            )));
        }

        // Check for critical errors
        // Note: The hyperlane_critical_error metric is set when the relayer loses
        // liveness on a chain, typically due to unreliable RPCs. We log this but
        // don't immediately fail - the message may still be processed.
        if metrics.has_critical_error().await.unwrap_or(false) {
            eprintln!("[WARN] Relayer reports critical_error=1 for a chain (possible RPC issue)");
            // Don't return early - let the timeout handle actual failures
        }

        // Check for error increase
        let current_errors = get_error_count_safe(metrics).await;
        if current_errors > initial_errors + config.max_error_increase {
            return Err(WaitError::TooManyErrors {
                initial: initial_errors,
                current: current_errors,
            });
        }

        // Check if we've reached the expected count
        match metrics.get_messages_processed_count(origin, remote).await {
            Ok(count) if count >= expected_count => {
                tracing::info!(
                    %count,
                    elapsed = ?start.elapsed(),
                    "Messages processed successfully"
                );
                return Ok(());
            }
            Ok(count) => {
                tracing::info!(%count, %expected_count, elapsed = ?start.elapsed(), "Waiting for more messages...");
            }
            Err(MetricsError::MetricNotFound(ref m)) => {
                tracing::info!(%m, elapsed = ?start.elapsed(), "Metric not found yet, waiting...");
            }
            Err(ref e) => {
                tracing::warn!(?e, elapsed = ?start.elapsed(), "Failed to fetch metrics, retrying...");
            }
        }

        tokio::time::sleep(config.poll_interval).await;
    }
}

/// Waits for all relayer queues to be empty.
///
/// This polls the `hyperlane_submitter_queue_length` metric for each queue type
/// until all are empty or times out.
#[allow(dead_code)]
pub async fn wait_for_queues_empty(
    metrics: &RelayerMetricsClient,
    config: RelayerWaitConfig,
) -> Result<(), WaitError> {
    let start = std::time::Instant::now();
    let initial_errors = get_error_count_safe(metrics).await;
    let queue_names = ["prepare_queue", "submit_queue", "confirm_queue"];

    tracing::info!(
        timeout_secs = ?config.timeout.as_secs(),
        "Waiting for relayer queues to empty"
    );

    loop {
        // Check for timeout
        if start.elapsed() > config.timeout {
            let queue_lengths: Vec<_> =
                futures::future::join_all(queue_names.iter().map(|q| metrics.get_queue_length(q)))
                    .await;
            return Err(WaitError::Timeout(format!(
                "Queue lengths after {:?}: {:?}",
                start.elapsed(),
                queue_names
                    .iter()
                    .zip(queue_lengths.iter())
                    .map(|(n, l)| format!(
                        "{n}={}",
                        l.as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|_| "?".into())
                    ))
                    .collect::<Vec<_>>()
            )));
        }

        // Check for critical errors
        if metrics.has_critical_error().await.unwrap_or(false) {
            return Err(WaitError::CriticalError);
        }

        // Check for error increase
        let current_errors = get_error_count_safe(metrics).await;
        if current_errors > initial_errors + config.max_error_increase {
            return Err(WaitError::TooManyErrors {
                initial: initial_errors,
                current: current_errors,
            });
        }

        // Check if all queues are empty
        let mut all_empty = true;
        for queue_name in &queue_names {
            match metrics.get_queue_length(queue_name).await {
                // Queue is empty or metric doesn't exist (treat as empty)
                Ok(0) | Err(MetricsError::MetricNotFound(_)) => {}
                Ok(len) => {
                    tracing::debug!(%queue_name, %len, "Queue not empty yet");
                    all_empty = false;
                }
                Err(e) => {
                    tracing::warn!(?e, %queue_name, "Failed to fetch queue length");
                }
            }
        }

        if all_empty {
            tracing::info!(elapsed = ?start.elapsed(), "All queues empty");
            return Ok(());
        }

        tokio::time::sleep(config.poll_interval).await;
    }
}

/// Gets the error count, returning 0 if metrics are unavailable.
async fn get_error_count_safe(metrics: &RelayerMetricsClient) -> u64 {
    metrics.get_error_count().await.unwrap_or(0)
}
