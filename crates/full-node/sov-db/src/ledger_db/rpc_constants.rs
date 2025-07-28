/// The maximum number of slots that can be requested in a single RPC range query
pub(crate) const MAX_SLOTS_PER_REQUEST: u64 = 10;
/// The maximum number of batches that can be requested in a single RPC range query
pub(crate) const MAX_BATCHES_PER_REQUEST: u64 = 20;
/// The maximum number of transactions that can be requested in a single RPC range query
pub(crate) const MAX_TRANSACTIONS_PER_REQUEST: u64 = 100;
/// The maximum number of events that can be requested in a single RPC range query
pub(crate) const MAX_EVENTS_PER_REQUEST: u64 = 500;
