use std::collections::HashMap;

use anyhow::Context;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// Global hashmap for storing metrics
pub static GLOBAL_HASHMAP: Lazy<Mutex<HashMap<String, (u64, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Adds metric to the global hashmap
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

/// Deserializes custom [`risc0_zkvm::Bytes`] into a tuple of (metric, value)
fn deserialize_custom(serialized: risc0_zkvm::Bytes) -> Result<(String, u64), anyhow::Error> {
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

/// Track metric provided as raw bytes. The bytes are expected to be a tuple of (metric, value)
pub fn metrics_callback(input: risc0_zkvm::Bytes) -> Result<risc0_zkvm::Bytes, anyhow::Error> {
    let met_tuple = deserialize_custom(input)?;
    add_value(met_tuple.0, met_tuple.1);
    Ok(risc0_zkvm::Bytes::new())
}
