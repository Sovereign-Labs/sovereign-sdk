mod merge;
mod parse;
mod rlp_chain;
mod schedule;
mod types;

use std::fs;
use std::path::Path;

use alloy_primitives::hex;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use self::merge::{build_evm_accounts_and_alloc_balances, merge_balances};
use self::parse::{ensure_object_field, normalize_hex_address, parse_optional_u64};
use self::rlp_chain::load_chain_timestamps;
use self::schedule::set_hardfork_schedule;
use self::types::{AdapterPaths, DEFAULT_BLOCK_GAS_LIMIT, DEFAULT_CHAIN_ID};

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

    let geth_genesis: Value = serde_json::from_slice(
        &fs::read(&paths.input_genesis)
            .with_context(|| format!("Failed to read {}", paths.input_genesis.display()))?,
    )
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

    let chain_timestamps = if let Some(chain_path) = paths.chain_rlp_path.as_ref() {
        if chain_path.exists() {
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
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let chain_id = parse_optional_u64(
        geth_genesis
            .get("config")
            .and_then(Value::as_object)
            .and_then(|c| c.get("chainId")),
        "config.chainId",
    )?
    .unwrap_or(DEFAULT_CHAIN_ID);

    let current_timestamp =
        parse_optional_u64(evm_genesis.get("genesis_timestamp"), "genesis_timestamp")?.unwrap_or(0);
    let genesis_timestamp = parse_optional_u64(geth_genesis.get("timestamp"), "timestamp")?
        .unwrap_or(current_timestamp);
    evm_genesis["genesis_timestamp"] = Value::from(genesis_timestamp);

    let current_base_fee =
        parse_optional_u64(evm_genesis.get("initial_base_fee"), "initial_base_fee")?.unwrap_or(0);
    let initial_base_fee = parse_optional_u64(geth_genesis.get("baseFeePerGas"), "baseFeePerGas")?
        .unwrap_or(current_base_fee);
    evm_genesis["initial_base_fee"] = Value::from(initial_base_fee);

    set_hardfork_schedule(&mut evm_genesis, &geth_genesis, &chain_timestamps)?;

    let chain_spec = ensure_object_field(&mut evm_genesis, "chain_spec")?;
    let existing_block_gas_limit = parse_optional_u64(
        chain_spec.get("block_gas_limit"),
        "chain_spec.block_gas_limit",
    )?
    .unwrap_or(DEFAULT_BLOCK_GAS_LIMIT);
    let block_gas_limit = parse_optional_u64(geth_genesis.get("gasLimit"), "gasLimit")?
        .unwrap_or(existing_block_gas_limit);
    chain_spec.insert("block_gas_limit".to_string(), Value::from(block_gas_limit));

    let mut tx_gas_limit =
        parse_optional_u64(chain_spec.get("tx_gas_limit"), "chain_spec.tx_gas_limit")?
            .unwrap_or(block_gas_limit);
    if tx_gas_limit > block_gas_limit {
        tx_gas_limit = block_gas_limit;
    }
    chain_spec.insert("tx_gas_limit".to_string(), Value::from(tx_gas_limit));

    if let Some(coinbase) = geth_genesis.get("coinbase").and_then(Value::as_str) {
        if coinbase.starts_with("0x") || coinbase.starts_with("0X") {
            if let Ok(address) = normalize_hex_address(coinbase) {
                chain_spec.insert(
                    "coinbase".to_string(),
                    Value::String(format!("0x{}", hex::encode(address.as_slice()))),
                );
            }
        }
    }

    let alloc = geth_genesis
        .get("alloc")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("geth genesis alloc must be an object"))?;

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
