use alloy_rpc_types::Filter;
use alloy_rpc_types::Log;
use derive_new::new;
use serde::Deserialize;
use serde::Serialize;

/// Filter for logs with cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FilterWithCursor {
    #[serde(flatten)]
    pub cursor: Option<String>,
    #[serde(flatten)]
    pub filter: Filter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, new)]
#[serde(rename_all = "camelCase")]
/// Logs and the next cursor.
pub struct LogsWithMaybeCursor {
    pub logs: Vec<Log>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
