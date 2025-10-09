use alloy_rpc_types::Filter;

/// Filter for logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FilterWithCursor {
    pub cursor: Option<u64>,
    pub filter: Filter,
}
