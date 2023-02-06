#[cfg(any(test, feature = "metrics"))]
pub mod counters;
#[cfg(any(test, feature = "metrics"))]
use counters::*;

#[cfg(any(test, feature = "metrics"))]
#[inline(always)]
/// Increment the counter `JELLYFISH_INTERNAL_ENCODED_BYTES` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_internal_encoded_bytes_if_enabled(amount: usize) {
    JELLYFISH_INTERNAL_ENCODED_BYTES.inc_by(amount as u64)
}

#[cfg(not(any(test, feature = "metrics")))]
#[inline(always)]
/// Increment the counter `JELLYFISH_INTERNAL_ENCODED_BYTES` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_internal_encoded_bytes_if_enabled(_amount: usize) {}

#[cfg(any(test, feature = "metrics"))]
#[inline(always)]
/// Increment the counter `JELLYFISH_INTERNAL_ENCODED_BYTES` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_leaf_encoded_bytes_if_enabled(amount: usize) {
    JELLYFISH_LEAF_ENCODED_BYTES.inc_by(amount as u64)
}

#[cfg(not(any(test, feature = "metrics")))]
#[inline(always)]
/// Increment the counter `JELLYFISH_INTERNAL_ENCODED_BYTES` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_leaf_encoded_bytes_if_enabled(_amount: usize) {}

#[cfg(any(test, feature = "metrics"))]
#[inline(always)]
/// Set the counter `JELLYFISH_LEAF_COUNT` to the provided count
/// if metrics are enabled. No-op otherwise
pub fn set_leaf_count_if_enabled(count: usize) {
    JELLYFISH_LEAF_COUNT.set(count as i64)
}

#[cfg(not(any(test, feature = "metrics")))]
#[inline(always)]
/// Set the counter `JELLYFISH_LEAF_COUNT` to the provided count
/// if metrics are enabled. No-op otherwise
pub fn set_leaf_count_if_enabled(_count: usize) {}

#[cfg(any(test, feature = "metrics"))]
#[inline(always)]
/// Increment the counter `JELLYFISH_LEAF_DELETION_COUNT` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_deletion_count_if_enabled(amount: usize) {
    JELLYFISH_LEAF_DELETION_COUNT.inc_by(amount as u64)
}

#[cfg(not(any(test, feature = "metrics")))]
#[inline(always)]
/// Increment the counter `JELLYFISH_LEAF_DELETION_COUNT` by amount
/// if metrics are enabled. No-op otherwise
pub fn inc_deletion_count_if_enabled(_amount: usize) {}
