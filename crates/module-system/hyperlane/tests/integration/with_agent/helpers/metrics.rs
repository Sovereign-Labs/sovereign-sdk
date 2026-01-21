//! Prometheus metrics client for monitoring the Hyperlane relayer.

use std::collections::HashMap;

/// Client for fetching and parsing Prometheus metrics from the relayer.
pub struct RelayerMetricsClient {
    base_url: String,
    client: reqwest::Client,
}

/// Error type for metrics operations.
#[derive(Debug)]
pub enum MetricsError {
    /// Failed to fetch metrics from the endpoint.
    FetchError(String),
    /// Failed to parse the metrics response.
    ParseError(String),
    /// The requested metric was not found.
    MetricNotFound(String),
}

impl std::fmt::Display for MetricsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricsError::FetchError(e) => write!(f, "Failed to fetch metrics: {e}"),
            MetricsError::ParseError(e) => write!(f, "Failed to parse metrics: {e}"),
            MetricsError::MetricNotFound(m) => write!(f, "Metric not found: {m}"),
        }
    }
}

impl std::error::Error for MetricsError {}

impl RelayerMetricsClient {
    /// Creates a new metrics client for the given host and port.
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            base_url: format!("http://{host}:{port}/metrics"),
            client: reqwest::Client::new(),
        }
    }

    /// Fetches raw metrics text from the Prometheus endpoint.
    pub async fn fetch_metrics(&self) -> Result<String, MetricsError> {
        self.client
            .get(&self.base_url)
            .send()
            .await
            .map_err(|e| MetricsError::FetchError(e.to_string()))?
            .text()
            .await
            .map_err(|e| MetricsError::FetchError(e.to_string()))
    }

    /// Gets the count of messages processed for a given origin/remote pair.
    ///
    /// Metric: `hyperlane_messages_processed_count{origin, remote}`
    /// This counter is incremented after a message is confirmed as delivered.
    pub async fn get_messages_processed_count(
        &self,
        origin: &str,
        remote: &str,
    ) -> Result<u64, MetricsError> {
        let metrics_text = self.fetch_metrics().await?;
        let labels = [("origin", origin), ("remote", remote)];
        parse_prometheus_metric(&metrics_text, "hyperlane_messages_processed_count", &labels)
            .map(|v| v as u64)
    }

    /// Gets the count of finalized transactions from Lander for a destination chain.
    ///
    /// Metric: `hyperlane_lander_finalized_transactions{destination=...}`
    pub async fn get_lander_finalized_transactions(
        &self,
        destination: &str,
    ) -> Result<u64, MetricsError> {
        let metrics_text = self.fetch_metrics().await?;
        let labels = [("destination", destination)];
        parse_prometheus_metric(
            &metrics_text,
            "hyperlane_lander_finalized_transactions",
            &labels,
        )
        .map(|v| v as u64)
    }

    /// Checks if the relayer has encountered a critical error on any chain.
    ///
    /// Metric: `hyperlane_critical_error{chain=...}`
    /// Returns true if any chain has critical_error > 0.
    pub async fn has_critical_error(&self) -> Result<bool, MetricsError> {
        let metrics_text = self.fetch_metrics().await?;
        // Check all critical_error metrics (one per chain)
        for line in metrics_text.lines() {
            if line.starts_with("hyperlane_critical_error{") {
                if let Some((_, value_str)) = line.rsplit_once(' ') {
                    if let Ok(value) = value_str.parse::<f64>() {
                        if value > 0.0 {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    /// Gets the count of error-level span events.
    ///
    /// Metric: `hyperlane_span_events_total{event_level="error"}`
    pub async fn get_error_count(&self) -> Result<u64, MetricsError> {
        let metrics_text = self.fetch_metrics().await?;
        let labels = [("event_level", "error")];
        match parse_prometheus_metric(&metrics_text, "hyperlane_span_events_total", &labels) {
            Ok(v) => Ok(v as u64),
            // If metric doesn't exist yet, no errors
            Err(MetricsError::MetricNotFound(_)) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

/// Parses a Prometheus metric value from the text format.
///
/// Handles the Prometheus exposition format:
/// ```text
/// # HELP metric_name Description
/// # TYPE metric_name gauge
/// metric_name{label1="value1",label2="value2"} 123.45
/// ```
fn parse_prometheus_metric(
    text: &str,
    metric_name: &str,
    labels: &[(&str, &str)],
) -> Result<f64, MetricsError> {
    for line in text.lines() {
        // Skip comments and empty lines
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        // Check if this line is for our metric
        if !line.starts_with(metric_name) {
            continue;
        }

        // Parse the line: metric_name{labels} value or metric_name value
        let (metric_with_labels, value_str) = match line.rsplit_once(' ') {
            Some((m, v)) => (m, v),
            None => continue,
        };

        // Check if labels match
        if labels_match(metric_with_labels, metric_name, labels) {
            return value_str
                .parse()
                .map_err(|e| MetricsError::ParseError(format!("Invalid value: {e}")));
        }
    }

    Err(MetricsError::MetricNotFound(format!(
        "{metric_name} with labels {labels:?}"
    )))
}

/// Checks if a metric line's labels match the expected labels.
fn labels_match(
    metric_with_labels: &str,
    metric_name: &str,
    expected_labels: &[(&str, &str)],
) -> bool {
    // No labels expected
    if expected_labels.is_empty() {
        return metric_with_labels == metric_name
            || metric_with_labels.starts_with(&format!("{metric_name}{{"));
    }

    // Extract labels from the metric line
    let labels_str = match metric_with_labels.strip_prefix(metric_name) {
        Some(rest) => rest,
        None => return false,
    };

    // Should be {label1="value1",label2="value2"}
    if !labels_str.starts_with('{') || !labels_str.ends_with('}') {
        return false;
    }

    let labels_inner = &labels_str[1..labels_str.len() - 1];
    let parsed_labels = parse_label_string(labels_inner);

    // Check all expected labels are present with correct values
    for (key, expected_value) in expected_labels {
        match parsed_labels.get(*key) {
            Some(actual_value) if actual_value == expected_value => {}
            _ => return false,
        }
    }

    true
}

/// Parses a Prometheus label string like `label1="value1",label2="value2"`.
fn parse_label_string(labels_str: &str) -> HashMap<&str, &str> {
    let mut labels = HashMap::new();

    // Simple parser that handles basic cases
    // Note: This doesn't handle escaped quotes in values
    let mut chars = labels_str.char_indices().peekable();

    while let Some((start, _)) = chars.next() {
        // Find the '='
        let eq_pos = labels_str[start..].find('=');
        let eq_pos = match eq_pos {
            Some(pos) => start + pos,
            None => break,
        };

        let key = labels_str[start..eq_pos].trim();

        // Skip past '='
        while chars.peek().map(|(i, _)| *i <= eq_pos).unwrap_or(false) {
            chars.next();
        }

        // Find the opening quote
        match chars.next() {
            Some((_, '"')) => {}
            _ => break,
        }

        // Find the closing quote
        let value_start = chars.peek().map(|(i, _)| *i).unwrap_or(labels_str.len());
        let mut value_end = value_start;

        for (i, c) in chars.by_ref() {
            if c == '"' {
                value_end = i;
                break;
            }
        }

        let value = &labels_str[value_start..value_end];
        labels.insert(key, value);

        // Skip comma if present
        if let Some((_, ',')) = chars.peek() {
            chars.next();
        }
    }

    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prometheus_metric_simple() {
        let text = r#"
# HELP hyperlane_messages_processed_count Number of messages processed
# TYPE hyperlane_messages_processed_count counter
hyperlane_messages_processed_count{origin="sovtest",remote="ethtest"} 5
hyperlane_messages_processed_count{origin="ethtest",remote="sovtest"} 3
"#;

        let result = parse_prometheus_metric(
            text,
            "hyperlane_messages_processed_count",
            &[("origin", "sovtest"), ("remote", "ethtest")],
        );
        assert_eq!(result.unwrap(), 5.0);

        let result = parse_prometheus_metric(
            text,
            "hyperlane_messages_processed_count",
            &[("origin", "ethtest"), ("remote", "sovtest")],
        );
        assert_eq!(result.unwrap(), 3.0);
    }

    #[test]
    fn test_parse_prometheus_metric_no_labels() {
        let text = r#"
# HELP hyperlane_critical_error Critical error indicator
# TYPE hyperlane_critical_error gauge
hyperlane_critical_error 0
"#;

        let result = parse_prometheus_metric(text, "hyperlane_critical_error", &[]);
        assert_eq!(result.unwrap(), 0.0);
    }

    #[test]
    fn test_parse_prometheus_metric_not_found() {
        let text = r#"
hyperlane_some_other_metric 42
"#;

        let result = parse_prometheus_metric(text, "hyperlane_critical_error", &[]);
        assert!(matches!(result, Err(MetricsError::MetricNotFound(_))));
    }

    #[test]
    fn test_parse_label_string() {
        let labels = parse_label_string(r#"origin="sovtest",remote="ethtest""#);
        assert_eq!(labels.get("origin"), Some(&"sovtest"));
        assert_eq!(labels.get("remote"), Some(&"ethtest"));
    }
}
