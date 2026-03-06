mod merge;
mod rlp_chain;
mod schedule;
mod types;

use std::fs;
use std::path::Path;

use alloy::genesis::Genesis;
use alloy_primitives::{hex, Address};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use self::merge::{build_evm_accounts_and_alloc_balances, merge_balances};
use self::rlp_chain::load_chain_timestamps;
use self::schedule::set_hardfork_schedule;
use self::types::{
    object_field_or_insert, value_as_optional_u64, AdapterPaths, DEFAULT_BLOCK_GAS_LIMIT,
    DEFAULT_CHAIN_ID,
};

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("Failed to create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("Failed to read {}", src.display()))? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn has_non_null_path(value: &Value, path: &[&str]) -> bool {
    let mut current = value;
    for segment in path {
        current = match current {
            Value::Object(map) => match map.get(*segment) {
                Some(next) => next,
                None => return false,
            },
            _ => return false,
        };
    }
    !current.is_null()
}

fn coinbase_override_and_sanitize(genesis_json: &mut Value) -> Option<Address> {
    let raw_coinbase = genesis_json.get("coinbase").and_then(Value::as_str)?;

    if raw_coinbase.starts_with("0x") || raw_coinbase.starts_with("0X") {
        if let Ok(address) = raw_coinbase.parse::<Address>() {
            return Some(address);
        }
    }

    if let Some(root) = genesis_json.as_object_mut() {
        root.remove("coinbase");
    }
    None
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let bin_name = args
        .first()
        .map_or("sov-hive-genesis-adapter", String::as_str);

    let paths = match AdapterPaths::parse(&args) {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("{}", AdapterPaths::usage(bin_name));
            return Err(err);
        }
    };

    if !paths.input_genesis.exists() {
        bail!("Missing genesis file: {}", paths.input_genesis.display());
    }
    if !paths.template_dir.exists() {
        bail!(
            "Missing template genesis directory: {}",
            paths.template_dir.display()
        );
    }

    if paths.output_dir.exists() {
        fs::remove_dir_all(&paths.output_dir)
            .with_context(|| format!("Failed to remove {}", paths.output_dir.display()))?;
    }
    copy_dir_all(&paths.template_dir, &paths.output_dir)?;

    let geth_genesis_bytes = fs::read(&paths.input_genesis)
        .with_context(|| format!("Failed to read {}", paths.input_genesis.display()))?;

    let mut geth_genesis_json: Value = serde_json::from_slice(&geth_genesis_bytes)
        .with_context(|| format!("Failed to parse {}", paths.input_genesis.display()))?;
    let has_chain_id_override = has_non_null_path(&geth_genesis_json, &["config", "chainId"]);
    let has_timestamp_override = has_non_null_path(&geth_genesis_json, &["timestamp"]);
    let has_base_fee_override = has_non_null_path(&geth_genesis_json, &["baseFeePerGas"]);
    let has_gas_limit_override = has_non_null_path(&geth_genesis_json, &["gasLimit"]);
    let has_config_override = geth_genesis_json
        .get("config")
        .is_some_and(Value::is_object);
    let coinbase_override = coinbase_override_and_sanitize(&mut geth_genesis_json);

    let geth_genesis: Genesis = serde_json::from_value(geth_genesis_json)
        .with_context(|| format!("Failed to parse {}", paths.input_genesis.display()))?;

    let evm_path = paths.output_dir.join("evm.json");
    let bank_path = paths.output_dir.join("bank.json");

    let mut evm_genesis: Value = serde_json::from_slice(
        &fs::read(&evm_path).with_context(|| format!("Failed to read {}", evm_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", evm_path.display()))?;
    let mut bank_genesis: Value = serde_json::from_slice(
        &fs::read(&bank_path).with_context(|| format!("Failed to read {}", bank_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", bank_path.display()))?;

    let chain_timestamps = paths
        .chain_rlp_path
        .as_deref()
        .filter(|chain_path| chain_path.exists())
        .map_or_else(Vec::new, |chain_path| {
            match load_chain_timestamps(chain_path) {
                Ok(ts) => ts,
                Err(err) => {
                    eprintln!(
                        "Warning: failed to parse {} for fork schedule: {err}",
                        chain_path.display()
                    );
                    Vec::new()
                }
            }
        });

    let chain_id = if has_chain_id_override {
        geth_genesis.config.chain_id
    } else {
        DEFAULT_CHAIN_ID
    };

    let current_timestamp =
        value_as_optional_u64(evm_genesis.get("genesis_timestamp"), "genesis_timestamp")?
            .unwrap_or(0);
    let genesis_timestamp = if has_timestamp_override {
        geth_genesis.timestamp
    } else {
        current_timestamp
    };
    evm_genesis["genesis_timestamp"] = Value::from(genesis_timestamp);

    let current_base_fee =
        value_as_optional_u64(evm_genesis.get("initial_base_fee"), "initial_base_fee")?
            .unwrap_or(0);
    let initial_base_fee = if has_base_fee_override {
        geth_genesis
            .base_fee_per_gas
            .map(|value| u64::try_from(value).context("baseFeePerGas exceeds u64"))
            .transpose()?
            .unwrap_or(current_base_fee)
    } else {
        current_base_fee
    };
    evm_genesis["initial_base_fee"] = Value::from(initial_base_fee);

    let hardfork_config = has_config_override.then_some(&geth_genesis.config);
    set_hardfork_schedule(&mut evm_genesis, hardfork_config, &chain_timestamps)?;

    let chain_spec = object_field_or_insert(&mut evm_genesis, "chain_spec")?;
    let existing_block_gas_limit = value_as_optional_u64(
        chain_spec.get("block_gas_limit"),
        "chain_spec.block_gas_limit",
    )?
    .unwrap_or(DEFAULT_BLOCK_GAS_LIMIT);
    let block_gas_limit = if has_gas_limit_override {
        geth_genesis.gas_limit
    } else {
        existing_block_gas_limit
    };
    chain_spec.insert("block_gas_limit".to_string(), Value::from(block_gas_limit));

    let tx_gas_limit =
        value_as_optional_u64(chain_spec.get("tx_gas_limit"), "chain_spec.tx_gas_limit")?
            .unwrap_or(block_gas_limit)
            .min(block_gas_limit);
    chain_spec.insert("tx_gas_limit".to_string(), Value::from(tx_gas_limit));

    if let Some(address) = coinbase_override {
        chain_spec.insert(
            "coinbase".to_string(),
            Value::String(format!("0x{}", hex::encode(address.as_slice()))),
        );
    }

    let alloc = &geth_genesis.alloc;

    let (evm_accounts, alloc_balances, alloc_stats) = build_evm_accounts_and_alloc_balances(alloc)?;
    evm_genesis["accounts"] = Value::Array(evm_accounts);

    let existing_balances = bank_genesis
        .get("gas_token_config")
        .and_then(Value::as_object)
        .and_then(|cfg| cfg.get("address_and_balances"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("bank.json missing gas_token_config.address_and_balances"))?
        .clone();

    let merged_balances = merge_balances(&existing_balances, alloc_balances)?;

    let bank_cfg = bank_genesis
        .get_mut("gas_token_config")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("bank.json missing gas_token_config object"))?;
    bank_cfg.insert(
        "address_and_balances".to_string(),
        Value::Array(merged_balances),
    );

    fs::write(
        &evm_path,
        format!("{}\n", serde_json::to_string_pretty(&evm_genesis)?),
    )
    .with_context(|| format!("Failed to write {}", evm_path.display()))?;
    fs::write(
        &bank_path,
        format!("{}\n", serde_json::to_string_pretty(&bank_genesis)?),
    )
    .with_context(|| format!("Failed to write {}", bank_path.display()))?;

    fs::write(
        paths.output_dir.join("chain_id.txt"),
        format!("{chain_id}\n"),
    )
    .with_context(|| {
        format!(
            "Failed to write {}/chain_id.txt",
            paths.output_dir.display()
        )
    })?;

    eprintln!(
        "Generated Hive genesis at {} (chain_id={}, alloc_accounts={}, contracts={}, storage_slots={})",
        paths.output_dir.display(),
        chain_id,
        alloc.len(),
        alloc_stats.contract_accounts,
        alloc_stats.storage_slots
    );

    Ok(())
}
