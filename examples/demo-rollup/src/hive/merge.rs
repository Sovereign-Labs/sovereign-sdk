use std::collections::{BTreeMap, HashMap};

use alloy_primitives::{hex, keccak256, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::types::{
    decode_hex_address, decode_u256_literal, value_as_u256, AllocStats, GethAllocAccount, U256Like,
};

type AddressBalance = (String, U256);
type AllocConversion = (Vec<Value>, Vec<AddressBalance>, AllocStats);

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

fn canonical_hex_data(raw: &str, field_name: &str) -> Result<String> {
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

fn to_quantity_hex(value: U256) -> String {
    format!("0x{value:x}")
}

fn normalize_storage(
    storage: Option<&BTreeMap<String, U256Like>>,
    account_name: &str,
) -> Result<serde_json::Map<String, Value>> {
    let mut normalized = serde_json::Map::new();
    let Some(storage) = storage else {
        return Ok(normalized);
    };
    for (raw_slot, raw_value) in storage {
        let slot = decode_u256_literal(raw_slot, &format!("alloc[{account_name}].storage slot"))?;
        let value = raw_value.to_u256(&format!("alloc[{account_name}].storage value"))?;
        normalized.insert(to_quantity_hex(slot), Value::String(to_quantity_hex(value)));
    }
    Ok(normalized)
}

pub(crate) fn build_evm_accounts_and_alloc_balances(
    alloc: &BTreeMap<String, GethAllocAccount>,
) -> Result<AllocConversion> {
    let mut evm_accounts = Vec::with_capacity(alloc.len());
    let mut alloc_balances: Vec<AddressBalance> = Vec::new();
    let mut stats = AllocStats::default();

    for (raw_address, entry) in alloc {
        let address = decode_hex_address(raw_address)?;
        let checksum_address = address.to_checksum(None);

        let code = canonical_hex_data(
            entry.code.as_deref().unwrap_or("0x"),
            &format!("alloc[{raw_address}].code"),
        )?;
        let code_bytes = hex::decode(&code[2..]).context("Failed to decode account code")?;
        let code_hash = format!("0x{}", hex::encode(keccak256(&code_bytes)));
        if !code_bytes.is_empty() {
            stats.contract_accounts += 1;
        }

        let nonce = entry
            .nonce
            .as_ref()
            .map(|value| value.to_u64(&format!("alloc[{raw_address}].nonce")))
            .transpose()?
            .unwrap_or(0);

        let storage_map = normalize_storage(entry.storage.as_ref(), raw_address)?;
        stats.storage_slots += storage_map.len();

        evm_accounts.push(json!({
            "address": checksum_address,
            "code_hash": code_hash,
            "code": code,
            "nonce": nonce,
            "storage": storage_map,
        }));

        let balance = entry
            .balance
            .as_ref()
            .map(|value| value.to_u256(&format!("alloc[{raw_address}].balance")))
            .transpose()?
            .unwrap_or(U256::ZERO);
        if balance > U256::ZERO {
            alloc_balances.push((address.to_checksum(None), balance));
        }
    }

    Ok((evm_accounts, alloc_balances, stats))
}

pub(crate) fn merge_balances(
    existing_balances: &[Value],
    alloc_balances: Vec<AddressBalance>,
) -> Result<Vec<Value>> {
    let mut merged: Vec<AddressBalance> = Vec::new();
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
        let amount = value_as_u256(&arr[1], &format!("bank.address_and_balances[{addr}]"))?;
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
        let alloc = serde_json::from_value::<BTreeMap<String, GethAllocAccount>>(json!({
            "0x0000000000000000000000000000000000000001": {
                "code": "0x6000",
                "storage": {"0x1": "0x2"},
                "balance": "0x1"
            },
            "0x0000000000000000000000000000000000000002": {
                "balance": "0x0"
            }
        }))
        .unwrap();
        let (accounts, balances, stats) = build_evm_accounts_and_alloc_balances(&alloc).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(balances.len(), 1);
        assert_eq!(stats.contract_accounts, 1);
        assert_eq!(stats.storage_slots, 1);
    }
}
