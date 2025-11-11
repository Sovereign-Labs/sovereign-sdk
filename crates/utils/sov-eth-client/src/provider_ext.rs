use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use alloy::transports::TransportResult;
use async_trait::async_trait;
use serde::Serialize;
use sov_rpc_eth_types::{LogWithExecutionTimestamp, LogsWithMaybeCursor};

#[derive(Debug, Clone, Serialize)]
pub struct LogsWithCursorParams {
    #[serde(flatten)]
    pub filter: Filter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Extension trait for custom eth_getLogsWithCursor RPC method
#[async_trait]
pub trait LogsWithCursorProvider: Provider {
    /// Fetches logs with pagination support using a cursor.
    ///
    /// # Arguments
    /// * `filter` - The log filter to apply
    /// * `cursor` - Optional cursor for pagination (None for first page)
    ///
    /// # Returns
    /// A response containing the logs and an optional cursor for the next page
    async fn get_logs_with_cursor(
        &self,
        filter: &Filter,
        cursor: Option<String>,
    ) -> TransportResult<LogsWithMaybeCursor>
    where
        Self: Sized,
    {
        let params = LogsWithCursorParams {
            filter: filter.clone(),
            cursor,
        };

        self.raw_request("eth_getLogsWithCursor".into(), [params])
            .await
    }

    /// Retrieves all logs matching the given filter using automatic pagination.
    ///
    /// This is a convenience method that handles pagination automatically by repeatedly
    /// calling `get_logs_with_cursor` until all logs are retrieved.
    ///
    /// # Arguments
    /// * `filter` - The log filter to apply
    ///
    /// # Returns
    /// A vector containing all logs matching the filter
    async fn get_all_logs_with_cursor(
        &self,
        filter: &Filter,
    ) -> TransportResult<Vec<LogWithExecutionTimestamp>>
    where
        Self: Sized,
    {
        let mut cursor = None;
        let mut all_logs = vec![];

        loop {
            let LogsWithMaybeCursor {
                logs: logs_page,
                cursor: next_cursor,
            } = self.get_logs_with_cursor(filter, cursor).await?;

            all_logs.extend(logs_page);

            if next_cursor.is_none() {
                break;
            }

            cursor = next_cursor;
        }

        Ok(all_logs)
    }
}

// Implement for all providers
#[async_trait]
impl<T: Provider + Sync> LogsWithCursorProvider for T {}
