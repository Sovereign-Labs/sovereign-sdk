//! Defines utilities for collecting runtime metrics from inside a Risc0 VM
use std::collections::HashMap;

use anyhow::Context;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use risc0_zkvm::Bytes;

/// A global hashmap mapping metric names to their values.
pub static GLOBAL_HASHMAP: Lazy<Mutex<HashMap<String, (u64, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Increments the requested metric by the given value, creating a
/// new entry in the global map if necessary.
fn add_value(metric: String, value: u64) {
    let mut hashmap = GLOBAL_HASHMAP.lock();
    hashmap
        .entry(metric)
        .and_modify(|(sum, count)| {
            *sum += value;
            *count += 1;
        })
        .or_insert((value, 1));
}

/// Deserialize a `Bytes` into a null-separated `(String, u64)` tuple. This function
/// expects its arguments to match the format of arguments to Risc0's io callbacks.
fn deserialize_custom(serialized: Bytes) -> Result<(String, u64), anyhow::Error> {
    let null_pos = serialized
        .iter()
        .position(|&b| b == 0)
        .context("Could not find separator in provided bytes")?;
    let (string_bytes, size_bytes_with_null) = serialized.split_at(null_pos);
    let size_bytes = &size_bytes_with_null[1..]; // Skip the null terminator
    let string = String::from_utf8(string_bytes.to_vec())?;
    let size = u64::from_ne_bytes(size_bytes.try_into()?); // Convert bytes back into usize
    Ok((string, size))
}

/// A custom callback for extracting metrics from the Risc0 zkvm.
///
/// When the "bench" feature is enabled, this callback is registered as a syscall
/// in the Risc0 VM and invoked whenever a function annotated with the [`sov-zk-cycle-utils::cycle_tracker`]
/// macro is invoked.
pub fn metrics_callback(input: risc0_zkvm::Bytes) -> Result<Bytes, anyhow::Error> {
    let met_tuple = deserialize_custom(input)?;
    add_value(met_tuple.0, met_tuple.1);
    Ok(Bytes::new())
}
