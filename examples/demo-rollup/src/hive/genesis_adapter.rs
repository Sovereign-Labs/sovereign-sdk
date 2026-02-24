use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_primitives::{hex, keccak256, Address, U256};
use alloy_rlp::{Decodable, Header, PayloadView};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

const DEFAULT_CHAIN_ID: u64 = 7;
const DEFAULT_BLOCK_GAS_LIMIT: u64 = 30_000_000;

fn usage(bin: &str) -> String {
    format!("Usage: {bin} <geth_genesis.json> <template_genesis_dir> <output_dir> [chain_rlp_path]")
}

fn parse_u64_literal(raw: &str, field_name: &str) -> Result<u64> {
    let s = raw.trim();
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if stripped.is_empty() {
            return Ok(0);
        }
        u64::from_str_radix(stripped, 16)
            .with_context(|| format!("Invalid hex integer for {field_name}: {raw}"))
    } else {
        s.parse::<u64>()
            .with_context(|| format!("Invalid integer for {field_name}: {raw}"))
    }
}

fn parse_u256_literal(raw: &str, field_name: &str) -> Result<U256> {
    let s = raw.trim();
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if stripped.is_empty() {
            return Ok(U256::ZERO);
        }
        U256::from_str_radix(stripped, 16)
            .with_context(|| format!("Invalid hex integer for {field_name}: {raw}"))
    } else {
        U256::from_str_radix(s, 10)
            .with_context(|| format!("Invalid integer for {field_name}: {raw}"))
    }
}

fn parse_u64_value(value: &Value, field_name: &str) -> Result<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow!("Invalid integer for {field_name}: {value}")),
        Value::String(s) => parse_u64_literal(s, field_name),
        _ => bail!("Invalid integer for {field_name}: {value}"),
    }
}

fn parse_optional_u64(value: Option<&Value>, field_name: &str) -> Result<Option<u64>> {
    match value {
        Some(Value::Null) | None => Ok(None),
        Some(v) => Ok(Some(parse_u64_value(v, field_name)?)),
    }
}

fn parse_u256_value(value: &Value, field_name: &str) -> Result<U256> {
    match value {
        Value::Number(n) => {
            let u = n
                .as_u64()
                .ok_or_else(|| anyhow!("Invalid integer for {field_name}: {value}"))?;
            Ok(U256::from(u))
        }
        Value::String(s) => parse_u256_literal(s, field_name),
        _ => bail!("Invalid integer for {field_name}: {value}"),
    }
}

fn normalize_hex_address(raw: &str) -> Result<Address> {
    let body = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if body.len() != 40 {
        bail!("Expected 20-byte address (40 hex chars), got {raw}");
    }
    if !body.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid hex address: {raw}");
    }
    let bytes = hex::decode(body).context("Failed to decode address hex")?;
    Ok(Address::from_slice(&bytes))
}

fn normalize_hex_data(raw: &str, field_name: &str) -> Result<String> {
    let body = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if body.is_empty() {
        return Ok("0x".to_string());
    }
    if !body.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid hex data for {field_name}: {raw}");
    }
    let normalized = if body.len() % 2 == 0 {
        body.to_ascii_lowercase()
    } else {
        format!("0{}", body.to_ascii_lowercase())
    };
    Ok(format!("0x{normalized}"))
}

fn to_hex_u256(value: U256) -> String {
    format!("0x{value:x}")
}

fn parse_storage_map(
    storage: &Map<String, Value>,
    account_name: &str,
) -> Result<Map<String, Value>> {
    let mut normalized = Map::new();
    for (raw_slot, raw_value) in storage {
        let slot = parse_u256_literal(raw_slot, &format!("alloc[{account_name}].storage slot"))?;
        let value = parse_u256_value(raw_value, &format!("alloc[{account_name}].storage value"))?;
        normalized.insert(to_hex_u256(slot), Value::String(to_hex_u256(value)));
    }
    Ok(normalized)
}

fn next_rlp_item<'a>(input: &mut &'a [u8], context: &str) -> Result<&'a [u8]> {
    if input.is_empty() {
        bail!("Unexpected end of RLP stream while decoding {context}");
    }

    let original = *input;
    let mut after_header = original;
    let header = Header::decode(&mut after_header)
        .with_context(|| format!("Failed to decode RLP header for {context}"))?;
    let header_len = original
        .len()
        .checked_sub(after_header.len())
        .ok_or_else(|| anyhow!("RLP length underflow"))?;
    let total_len = header_len
        .checked_add(header.payload_length)
        .ok_or_else(|| anyhow!("RLP length overflow"))?;
    if total_len > original.len() {
        bail!("RLP item for {context} extends beyond available input");
    }

    let (item, rest) = original.split_at(total_len);
    *input = rest;
    Ok(item)
}

fn decode_rlp_list_items<'a>(raw: &'a [u8], context: &str) -> Result<Vec<&'a [u8]>> {
    let mut cursor = raw;
    let payload = Header::decode_raw(&mut cursor)
        .with_context(|| format!("Failed to decode RLP payload for {context}"))?;
    if !cursor.is_empty() {
        bail!("Malformed RLP list for {context}: trailing bytes");
    }
    match payload {
        PayloadView::List(items) => Ok(items),
        PayloadView::String(_) => bail!("Expected RLP list for {context}"),
    }
}

fn decode_rlp_u64(raw: &[u8], context: &str) -> Result<u64> {
    let mut field = raw;
    let value = u64::decode(&mut field)
        .with_context(|| format!("Failed to decode RLP quantity for {context}"))?;
    if !field.is_empty() {
        bail!("Malformed RLP quantity for {context}: trailing bytes");
    }
    Ok(value)
}

fn load_chain_timestamps(chain_rlp_path: &Path) -> Result<Vec<(u64, u64)>> {
    let data = fs::read(chain_rlp_path)
        .with_context(|| format!("Failed to read {}", chain_rlp_path.display()))?;
    let mut out = Vec::new();
    let mut cursor = data.as_slice();

    while !cursor.is_empty() {
        let block_item = next_rlp_item(&mut cursor, "chain.rlp block")?;
        let block_fields = decode_rlp_list_items(block_item, "chain.rlp block")?;
        if block_fields.is_empty() {
            bail!("Malformed block in chain.rlp");
        }

        let header_fields = decode_rlp_list_items(block_fields[0], "chain.rlp block header")?;
        if header_fields.len() < 12 {
            bail!("Block header has fewer fields than expected");
        }

        let block_number = decode_rlp_u64(header_fields[8], "chain.rlp block number")?;
        let timestamp = decode_rlp_u64(header_fields[11], "chain.rlp block timestamp")?;
        out.push((block_number, timestamp));
    }

    Ok(out)
}

fn activation_block_for_timestamp(chain_timestamps: &[(u64, u64)], timestamp: u64) -> Option<u64> {
    chain_timestamps
        .iter()
        .find_map(|(block_number, block_ts)| (*block_ts >= timestamp).then_some(*block_number))
}

fn push_block_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    config: &Map<String, Value>,
    field: &str,
    fork_name: &'static str,
) -> Result<()> {
    if let Some(block) = parse_optional_u64(config.get(field), field)? {
        schedule.push((block, fork_name));
    }
    Ok(())
}

fn push_time_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    config: &Map<String, Value>,
    field: &str,
    fork_name: &'static str,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    if let Some(timestamp) = parse_optional_u64(config.get(field), field)? {
        if let Some(block) = activation_block_for_timestamp(chain_timestamps, timestamp) {
            schedule.push((block, fork_name));
        }
    }
    Ok(())
}

fn set_hardfork_schedule(
    evm_genesis: &mut Value,
    geth_genesis: &Value,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    let Some(config) = geth_genesis.get("config").and_then(Value::as_object) else {
        return Ok(());
    };

    let mut schedule: Vec<(u64, &'static str)> = vec![(0, "FRONTIER")];

    push_block_fork(&mut schedule, config, "homesteadBlock", "HOMESTEAD")?;
    push_block_fork(&mut schedule, config, "eip150Block", "TANGERINE")?;

    let eip155 = parse_optional_u64(config.get("eip155Block"), "eip155Block")?;
    let eip158 = parse_optional_u64(config.get("eip158Block"), "eip158Block")?;
    if let Some(spurious_block) = [eip155, eip158].into_iter().flatten().max() {
        schedule.push((spurious_block, "SPURIOUS_DRAGON"));
    }

    push_block_fork(&mut schedule, config, "byzantiumBlock", "BYZANTIUM")?;
    push_block_fork(
        &mut schedule,
        config,
        "constantinopleBlock",
        "CONSTANTINOPLE",
    )?;
    push_block_fork(&mut schedule, config, "petersburgBlock", "PETERSBURG")?;
    push_block_fork(&mut schedule, config, "istanbulBlock", "ISTANBUL")?;
    push_block_fork(&mut schedule, config, "muirGlacierBlock", "MUIR_GLACIER")?;
    push_block_fork(&mut schedule, config, "berlinBlock", "BERLIN")?;
    push_block_fork(&mut schedule, config, "londonBlock", "LONDON")?;
    push_block_fork(&mut schedule, config, "arrowGlacierBlock", "ARROW_GLACIER")?;
    push_block_fork(&mut schedule, config, "grayGlacierBlock", "GRAY_GLACIER")?;
    push_block_fork(&mut schedule, config, "mergeNetsplitBlock", "MERGE")?;

    push_time_fork(
        &mut schedule,
        config,
        "shanghaiTime",
        "SHANGHAI",
        chain_timestamps,
    )?;
    push_time_fork(
        &mut schedule,
        config,
        "cancunTime",
        "CANCUN",
        chain_timestamps,
    )?;
    push_time_fork(
        &mut schedule,
        config,
        "pragueTime",
        "PRAGUE",
        chain_timestamps,
    )?;

    schedule.sort_by_key(|(block, _)| *block);
    let mut deduped: Vec<(u64, &'static str)> = Vec::with_capacity(schedule.len());
    for (block, fork_name) in schedule {
        if let Some((last_block, last_name)) = deduped.last_mut() {
            if *last_block == block {
                *last_name = fork_name;
                continue;
            }
        }
        deduped.push((block, fork_name));
    }

    let chain_spec = ensure_object_field(evm_genesis, "chain_spec")?;
    let hardforks = deduped
        .into_iter()
        .map(|(block, fork)| {
            Value::Array(vec![Value::from(block), Value::String(fork.to_string())])
        })
        .collect::<Vec<_>>();
    chain_spec.insert("hardforks".to_string(), Value::Array(hardforks));
    Ok(())
}

fn ensure_object_field<'a>(parent: &'a mut Value, key: &str) -> Result<&'a mut Map<String, Value>> {
    let parent_obj = parent
        .as_object_mut()
        .ok_or_else(|| anyhow!("Expected JSON object at root"))?;
    if !matches!(parent_obj.get(key), Some(Value::Object(_))) {
        parent_obj.insert(key.to_string(), Value::Object(Map::new()));
    }
    parent_obj
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("Expected object field '{key}'"))
}

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

fn balance_key(address: &str) -> String {
    if address.starts_with("0x") || address.starts_with("0X") {
        address.to_ascii_lowercase()
    } else {
        address.to_string()
    }
}

fn add_balance(
    merged: &mut Vec<(String, U256)>,
    index_by_key: &mut HashMap<String, usize>,
    address: String,
    amount: U256,
) -> Result<()> {
    let key = balance_key(&address);
    if let Some(idx) = index_by_key.get(&key).copied() {
        merged[idx].1 = merged[idx]
            .1
            .checked_add(amount)
            .ok_or_else(|| anyhow!("Balance overflow while merging {address}"))?;
        return Ok(());
    }
    index_by_key.insert(key, merged.len());
    merged.push((address, amount));
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 && args.len() != 5 {
        eprintln!(
            "{}",
            usage(
                args.first()
                    .map_or("sov-hive-genesis-adapter", String::as_str)
            )
        );
        bail!("Invalid arguments");
    }

    let input_genesis = PathBuf::from(&args[1]);
    let template_dir = PathBuf::from(&args[2]);
    let output_dir = PathBuf::from(&args[3]);
    let chain_rlp_path = args.get(4).map(PathBuf::from);

    if !input_genesis.exists() {
        bail!("Missing genesis file: {}", input_genesis.display());
    }
    if !template_dir.exists() {
        bail!(
            "Missing template genesis directory: {}",
            template_dir.display()
        );
    }

    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)
            .with_context(|| format!("Failed to remove {}", output_dir.display()))?;
    }
    copy_dir_all(&template_dir, &output_dir)?;

    let geth_genesis: Value = serde_json::from_slice(
        &fs::read(&input_genesis)
            .with_context(|| format!("Failed to read {}", input_genesis.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", input_genesis.display()))?;

    let evm_path = output_dir.join("evm.json");
    let bank_path = output_dir.join("bank.json");

    let mut evm_genesis: Value = serde_json::from_slice(
        &fs::read(&evm_path).with_context(|| format!("Failed to read {}", evm_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", evm_path.display()))?;
    let mut bank_genesis: Value = serde_json::from_slice(
        &fs::read(&bank_path).with_context(|| format!("Failed to read {}", bank_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", bank_path.display()))?;

    let chain_timestamps = if let Some(chain_path) = chain_rlp_path.as_ref() {
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

    let mut evm_accounts = Vec::with_capacity(alloc.len());
    let mut alloc_balances: Vec<(String, U256)> = Vec::new();
    let mut contract_accounts = 0usize;
    let mut storage_slots = 0usize;

    for (raw_address, alloc_entry) in alloc {
        let address = normalize_hex_address(raw_address)?;
        let checksum_address = address.to_checksum(None);
        let entry = alloc_entry.as_object();

        let code = normalize_hex_data(
            entry
                .and_then(|obj| obj.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("0x"),
            &format!("alloc[{raw_address}].code"),
        )?;
        let code_bytes = hex::decode(&code[2..]).context("Failed to decode account code")?;
        let code_hash = format!("0x{}", hex::encode(keccak256(&code_bytes)));
        if !code_bytes.is_empty() {
            contract_accounts += 1;
        }

        let nonce = parse_optional_u64(
            entry.and_then(|obj| obj.get("nonce")),
            &format!("alloc[{raw_address}].nonce"),
        )?
        .unwrap_or(0);

        let storage_value = entry.and_then(|obj| obj.get("storage"));
        let storage_map = match storage_value {
            Some(Value::Object(storage)) => parse_storage_map(storage, raw_address)?,
            Some(Value::Null) | None => Map::new(),
            Some(_) => bail!("alloc[{raw_address}].storage must be an object"),
        };
        storage_slots += storage_map.len();

        evm_accounts.push(json!({
            "address": checksum_address,
            "code_hash": code_hash,
            "code": code,
            "nonce": nonce,
            "storage": storage_map,
        }));

        let balance = parse_u256_value(
            entry
                .and_then(|obj| obj.get("balance"))
                .unwrap_or(&Value::String("0x0".to_string())),
            &format!("alloc[{raw_address}].balance"),
        )?;
        if balance > U256::ZERO {
            alloc_balances.push((address.to_checksum(None), balance));
        }
    }

    evm_genesis["accounts"] = Value::Array(evm_accounts);

    let existing_balances = bank_genesis
        .get("gas_token_config")
        .and_then(Value::as_object)
        .and_then(|cfg| cfg.get("address_and_balances"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("bank.json missing gas_token_config.address_and_balances"))?
        .clone();

    let mut merged: Vec<(String, U256)> = Vec::new();
    let mut index_by_key: HashMap<String, usize> = HashMap::new();

    for pair in existing_balances {
        let arr = pair
            .as_array()
            .ok_or_else(|| anyhow!("Invalid bank balance entry: expected [address, amount]"))?;
        if arr.len() != 2 {
            bail!("Invalid bank balance entry length: expected 2");
        }
        let addr = arr[0]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid bank balance address"))?
            .to_string();
        let amount = parse_u256_value(&arr[1], &format!("bank.address_and_balances[{addr}]"))?;
        add_balance(&mut merged, &mut index_by_key, addr, amount)?;
    }

    for (addr, amount) in alloc_balances {
        add_balance(&mut merged, &mut index_by_key, addr, amount)?;
    }

    let merged_balances = merged
        .into_iter()
        .map(|(addr, amount)| {
            Value::Array(vec![Value::String(addr), Value::String(amount.to_string())])
        })
        .collect::<Vec<_>>();

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

    fs::write(output_dir.join("chain_id.txt"), format!("{chain_id}\n"))
        .with_context(|| format!("Failed to write {}/chain_id.txt", output_dir.display()))?;

    eprintln!(
        "Generated Hive genesis at {} (chain_id={}, alloc_accounts={}, contracts={}, storage_slots={})",
        output_dir.display(),
        chain_id,
        alloc.len(),
        contract_accounts,
        storage_slots
    );

    Ok(())
}
