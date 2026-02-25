use std::collections::HashMap;

use alloy_primitives::{hex, keccak256, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::parse::{
    normalize_hex_address, normalize_hex_data, parse_optional_u64, parse_storage_map,
    parse_u256_value,
};
use crate::types::AllocStats;

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

pub(crate) fn build_evm_accounts_and_alloc_balances(
    alloc: &Map<String, Value>,
) -> Result<(Vec<Value>, Vec<(String, U256)>, AllocStats)> {
    let mut evm_accounts = Vec::with_capacity(alloc.len());
    let mut alloc_balances: Vec<(String, U256)> = Vec::new();
    let mut stats = AllocStats::default();

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
            stats.contract_accounts += 1;
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
        stats.storage_slots += storage_map.len();

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

    Ok((evm_accounts, alloc_balances, stats))
}

pub(crate) fn merge_balances(
    existing_balances: &[Value],
    alloc_balances: Vec<(String, U256)>,
) -> Result<Vec<Value>> {
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

    Ok(merged
        .into_iter()
        .map(|(addr, amount)| {
            Value::Array(vec![Value::String(addr), Value::String(amount.to_string())])
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_balances_adds_and_merges_case_insensitive_hex() {
        let existing = vec![json!(["0xAbCd", "10"])];
        let alloc = vec![("0xabcd".to_string(), U256::from(5u64))];
        let merged = merge_balances(&existing, alloc).unwrap();
        assert_eq!(merged, vec![json!(["0xAbCd", "15"])]);
    }

    #[test]
    fn merge_balances_rejects_overflow() {
        let existing = vec![json!(["0xabcd", U256::MAX.to_string()])];
        let alloc = vec![("0xabcd".to_string(), U256::from(1u64))];
        assert!(merge_balances(&existing, alloc).is_err());
    }

    #[test]
    fn build_evm_accounts_counts_contracts_and_storage() {
        let alloc = json!({
            "0x0000000000000000000000000000000000000001": {
                "code": "0x6000",
                "storage": {"0x1": "0x2"},
                "balance": "0x1"
            },
            "0x0000000000000000000000000000000000000002": {
                "balance": "0x0"
            }
        });
        let alloc_obj = alloc.as_object().unwrap();
        let (accounts, balances, stats) = build_evm_accounts_and_alloc_balances(alloc_obj).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(balances.len(), 1);
        assert_eq!(stats.contract_accounts, 1);
        assert_eq!(stats.storage_slots, 1);
    }
}
