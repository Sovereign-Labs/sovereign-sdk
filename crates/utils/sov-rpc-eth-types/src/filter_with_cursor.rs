use alloy_primitives::Address;
use alloy_rpc_types::Topic;
use alloy_rpc_types::{FilterBlockOption, FilterSet};

/// Filter for logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FilterWithCursor {
    pub cursor: Option<u64>,
    pub block_option: FilterBlockOption,
    pub address: FilterSet<Address>,
    pub topics: [Topic; 4],
}
