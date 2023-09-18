use std::collections::HashMap;

use anyhow::Context;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use risc0_zkvm::Bytes;

pub static GLOBAL_HASHMAP: Lazy<Mutex<HashMap<String, (u64, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn add_value(metric: String, value: u64) {
    let mut hashmap = GLOBAL_HASHMAP.lock();
    hashmap
        .entry(metric)
        .and_modify(|(sum, count)| {
            *sum += value;
            *count += 1;
        })
        .or_insert((value, 1));
}

pub fn deserialize_custom(serialized: Bytes) -> Result<(String, u64), anyhow::Error> {
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

pub fn metrics_callback(input: risc0_zkvm::Bytes) -> Result<Bytes, anyhow::Error> {
    let met_tuple = deserialize_custom(input)?;
    add_value(met_tuple.0, met_tuple.1);
    Ok(Bytes::new())
}
