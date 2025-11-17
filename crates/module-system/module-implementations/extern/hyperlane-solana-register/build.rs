use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::Path;

const DEFAULT_NETWORK: &str = "testnet";

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let build_script_dir = Path::new(&manifest_dir);
    let config_path = build_script_dir.join("configs.json");
    let json_str = std::fs::read_to_string(config_path).unwrap();
    let json: Value = serde_json::from_str(&json_str).unwrap();

    let network = std::env::var("SOV_HYPERLANE_SOLANA_NETWORK").unwrap_or_else(|_| {
        println!("cargo:warning=SOV_HYPERLANE_SOLANA_NETWORK env var not set, defaulting to solana {DEFAULT_NETWORK}");
        DEFAULT_NETWORK.to_string()
    });
    let network = match network.as_str() {
        "mainnet" | "testnet" => network,
        _ => {
            println!(
                "cargo:warning=SOV_HYPERLANE_SOLANA_NETWORK set to invalid value {network}, using '{DEFAULT_NETWORK}' (valid values: 'mainnet', 'testnet')",
            );
            DEFAULT_NETWORK.to_string()
        }
    };

    let config = json[&network].clone();
    let chain_id = config["chainId"].as_u64().unwrap();
    let program_id = config["programId"].as_str().unwrap();

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("config.rs");
    let mut f = File::create(&dest_path).unwrap();

    writeln!(
        &mut f,
        "pub const HYPERLANE_SOLANA_CHAIN_ID: u32 = {chain_id};",
    )
    .unwrap();
    writeln!(
        &mut f,
        "pub const SOLANA_PROGRAM_ID: &str = \"{program_id}\";",
    )
    .unwrap();
}
