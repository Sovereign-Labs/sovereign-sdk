use alloy_rpc_types::Filter;
use alloy_rpc_types::Log;

/// Filter for logs with cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FilterWithCursor {
    pub cursor: Option<u128>,
    pub filter: Filter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
/// Logs and the next cursor.
pub struct LogsWithMaybeCursor {
    pub logs: Vec<Log>,
    pub cursor: Option<u128>,
}
