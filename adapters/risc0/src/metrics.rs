use std::collections::HashMap;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use risc0_zkvm_platform::syscall::SyscallName;

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

pub fn deserialize_custom(serialized: &[u8]) -> (String, u64) {
    let null_pos = serialized.iter().position(|&b| b == 0).unwrap();
    let (string_bytes, size_bytes_with_null) = serialized.split_at(null_pos);
    let size_bytes = &size_bytes_with_null[1..]; // Skip the null terminator
    let string = String::from_utf8(string_bytes.to_vec()).unwrap();
    let size = u64::from_ne_bytes(size_bytes.try_into().unwrap()); // Convert bytes back into usize
    (string, size)
}

pub fn metrics_callback(input: &[u8]) -> Vec<u8> {
    let met_tuple = deserialize_custom(input);
    add_value(met_tuple.0, met_tuple.1);
    vec![]
}

pub fn get_syscall_name() -> SyscallName {
    let cycle_string = "cycle_metrics\0";
    let bytes = cycle_string.as_bytes();
    unsafe { SyscallName::from_bytes_with_nul(bytes.as_ptr()) }
}
