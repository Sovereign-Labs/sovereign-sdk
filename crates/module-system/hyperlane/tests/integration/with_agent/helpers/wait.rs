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
    /// Relayer reported a critical error.
    CriticalError,
    /// Relayer encountered too many errors.
    TooManyErrors { initial: u64, current: u64 },
    /// Failed to fetch metrics.
    MetricsError(MetricsError),
}

impl std::fmt::Display for WaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitError::Timeout(msg) => write!(f, "Timeout: {msg}"),
            WaitError::CriticalError => write!(f, "Relayer reported a critical error"),
            WaitError::TooManyErrors { initial, current } => {
                write!(
                    f,
                    "Relayer error count increased from {initial} to {current}"
                )
            }
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
    let initial_messages = get_messages_processed_count_safe(metrics, origin, remote).await;
    let initial_finalized = get_lander_finalized_transactions_safe(metrics, remote).await;
    let target_messages = initial_messages.saturating_add(expected_count);
    let target_finalized = initial_finalized.saturating_add(expected_count);

    tracing::info!(
        %origin,
        %remote,
        %expected_count,
        timeout_secs = ?config.timeout.as_secs(),
        "Waiting for messages to be processed"
    );

    // hyperlane_messages_processed_count increments only after confirmation, which is delayed
    // ~10 minutes in production builds. Lander finalized transactions are a faster signal.
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
            let current_messages = get_messages_processed_count_safe(metrics, origin, remote).await;
            let current_finalized = get_lander_finalized_transactions_safe(metrics, remote).await;
            return Err(WaitError::Timeout(format!(
                "Messages processed: {current_messages}/{target_messages}, \
                 lander finalized txs: {current_finalized}/{target_finalized} after {:?}",
                start.elapsed()
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

        let current_messages = get_messages_processed_count_safe(metrics, origin, remote).await;
        let current_finalized = get_lander_finalized_transactions_safe(metrics, remote).await;
        if current_messages >= target_messages || current_finalized >= target_finalized {
            tracing::info!(
                %current_messages,
                %current_finalized,
                elapsed = ?start.elapsed(),
                "Relayer progress detected"
            );
            return Ok(());
        }

        tracing::info!(
            %current_messages,
            %target_messages,
            %current_finalized,
            %target_finalized,
            elapsed = ?start.elapsed(),
            "Waiting for relayer progress..."
        );

        tokio::time::sleep(config.poll_interval).await;
    }
}

/// Gets the error count, returning 0 if metrics are unavailable.
async fn get_error_count_safe(metrics: &RelayerMetricsClient) -> u64 {
    metrics.get_error_count().await.unwrap_or(0)
}

async fn get_messages_processed_count_safe(
    metrics: &RelayerMetricsClient,
    origin: &str,
    remote: &str,
) -> u64 {
    metrics
        .get_messages_processed_count(origin, remote)
        .await
        .unwrap_or(0)
}

async fn get_lander_finalized_transactions_safe(
    metrics: &RelayerMetricsClient,
    destination: &str,
) -> u64 {
    metrics
        .get_lander_finalized_transactions(destination)
        .await
        .unwrap_or(0)
}
