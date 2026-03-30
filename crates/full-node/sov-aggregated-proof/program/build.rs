use std::env;
use std::fs;
use std::path::PathBuf;

use sp1_sdk::prelude::HashableKey;
use sp1_sdk::SP1VerifyingKey;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let data_dir = manifest_dir
        .parent()
        .expect("program crate must live under the sov-aggregated-proof workspace root")
        .join("data");
    let inner_vk_path = data_dir.join("inner_vk.bin");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    println!("cargo:rerun-if-changed={}", inner_vk_path.display());

    let inner_vk_bytes = fs::read(&inner_vk_path).unwrap_or_else(|error| {
        panic!(
            "Failed to read saved inner verifying key fixture at {}: {error}",
            inner_vk_path.display()
        )
    });
    let inner_vk: SP1VerifyingKey = bincode::deserialize(&inner_vk_bytes).unwrap_or_else(|error| {
        panic!(
            "Failed to deserialize saved inner verifying key fixture at {}: {error}",
            inner_vk_path.display()
        )
    });
    let inner_vk_hash = inner_vk.hash_u32();

    let generated = format!("const INNER_VKEY_HASH: [u32; 8] = {inner_vk_hash:?};\n");

    fs::write(out_dir.join("inner_vk_hash.rs"), generated)
        .expect("Failed to write generated inner VK hash file");
}
