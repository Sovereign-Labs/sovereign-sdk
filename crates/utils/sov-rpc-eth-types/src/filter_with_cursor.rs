use alloy_primitives::{serde_hex, Address};
use alloy_rpc_types::{Filter, Topic};
use alloy_rpc_types::{FilterBlockOption, FilterSet};

/// Filter for logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FilterWithCursor {
    pub cursor: Option<u64>,
    pub filter: Filter,
}
