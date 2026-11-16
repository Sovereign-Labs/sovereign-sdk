use crate::LogWithExecutionTimestamp;
use alloy_rpc_types::Filter;
use serde::Deserialize;
use serde::Serialize;

/// Filter for logs with cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FilterWithCursor {
    pub cursor: Option<String>,
    #[serde(flatten)]
    pub filter: Filter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Logs and the next cursor.
pub struct LogsWithMaybeCursor {
    pub logs: Vec<LogWithExecutionTimestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl LogsWithMaybeCursor {
    /// Creates a page of logs; `cursor` points at the next page, if any.
    pub fn new(logs: Vec<LogWithExecutionTimestamp>, cursor: Option<String>) -> Self {
        Self { logs, cursor }
    }
}
