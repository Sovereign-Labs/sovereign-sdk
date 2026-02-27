use std::collections::{BTreeMap, HashMap};

use alloy::genesis::GenesisAccount;
use alloy_primitives::{hex, keccak256, Address, B256, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::types::AllocStats;

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

fn decode_u256_literal(raw: &str, field_name: &str) -> Result<U256> {
    let value = raw.trim();
    if let Some(stripped) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if stripped.is_empty() {
            return Ok(U256::ZERO);
        }
        return U256::from_str_radix(stripped, 16)
            .with_context(|| format!("Invalid hex integer for {field_name}: {raw}"));
    }
    U256::from_str_radix(value, 10)
        .with_context(|| format!("Invalid integer for {field_name}: {raw}"))
}

fn value_as_u256(value: &Value, field_name: &str) -> Result<U256> {
    if let Some(value) = value.as_u64() {
        return Ok(U256::from(value));
    }
    if let Some(value) = value.as_str() {
        return decode_u256_literal(value, field_name);
    }

    bail!("Invalid integer for {field_name}: {value}")
}

fn to_quantity_hex(value: U256) -> String {
    format!("0x{value:x}")
}

fn normalize_storage(storage: Option<&BTreeMap<B256, B256>>) -> serde_json::Map<String, Value> {
    let mut normalized = serde_json::Map::new();
    let Some(storage) = storage else {
        return normalized;
    };

    for (raw_slot, raw_value) in storage {
        let slot = U256::from_be_slice(raw_slot.as_slice());
        let value = U256::from_be_slice(raw_value.as_slice());
        normalized.insert(to_quantity_hex(slot), Value::String(to_quantity_hex(value)));
    }

    normalized
}

pub(crate) fn build_evm_accounts_and_alloc_balances(
    alloc: &BTreeMap<Address, GenesisAccount>,
) -> Result<AllocConversion> {
    let mut evm_accounts = Vec::with_capacity(alloc.len());
    let mut alloc_balances: Vec<AddressBalance> = Vec::new();
    let mut stats = AllocStats::default();

    for (address, entry) in alloc {
        let checksum_address = address.to_checksum(None);

        let code_bytes = entry.code.as_deref().map_or(&[][..], |code| code);
        let code = if code_bytes.is_empty() {
            "0x".to_string()
        } else {
            format!("0x{}", hex::encode(code_bytes))
        };
        let code_hash = format!("0x{}", hex::encode(keccak256(code_bytes)));
        if !code_bytes.is_empty() {
            stats.contract_accounts += 1;
        }

        let nonce = entry.nonce.unwrap_or(0);

        let storage_map = normalize_storage(entry.storage.as_ref());
        stats.storage_slots += storage_map.len();

        evm_accounts.push(json!({
            "address": checksum_address,
            "code_hash": code_hash,
            "code": code,
            "nonce": nonce,
            "storage": storage_map,
        }));

        let balance = entry.balance;
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
        let alloc = serde_json::from_value::<BTreeMap<Address, GenesisAccount>>(json!({
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
