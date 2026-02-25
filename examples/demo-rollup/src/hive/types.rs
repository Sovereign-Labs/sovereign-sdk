use std::collections::BTreeMap;
use std::path::PathBuf;

use alloy_primitives::{hex, Address, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{Map, Value};

pub(crate) const DEFAULT_CHAIN_ID: u64 = 7;
pub(crate) const DEFAULT_BLOCK_GAS_LIMIT: u64 = 30_000_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AllocStats {
    pub(crate) contract_accounts: usize,
    pub(crate) storage_slots: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct AdapterPaths {
    pub(crate) input_genesis: PathBuf,
    pub(crate) template_dir: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) chain_rlp_path: Option<PathBuf>,
}

impl AdapterPaths {
    pub(crate) fn usage(bin: &str) -> String {
        format!(
            "Usage: {bin} <geth_genesis.json> <template_genesis_dir> <output_dir> [chain_rlp_path]"
        )
    }

    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        if args.len() != 4 && args.len() != 5 {
            bail!("Invalid arguments");
        }

        Ok(Self {
            input_genesis: PathBuf::from(&args[1]),
            template_dir: PathBuf::from(&args[2]),
            output_dir: PathBuf::from(&args[3]),
            chain_rlp_path: args.get(4).map(PathBuf::from),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GethGenesis {
    #[serde(default)]
    pub(crate) config: Option<GethConfig>,
    #[serde(default)]
    pub(crate) timestamp: Option<U64Like>,
    #[serde(default, rename = "baseFeePerGas")]
    pub(crate) base_fee_per_gas: Option<U64Like>,
    #[serde(default, rename = "gasLimit")]
    pub(crate) gas_limit: Option<U64Like>,
    #[serde(default)]
    pub(crate) coinbase: Option<String>,
    pub(crate) alloc: BTreeMap<String, GethAllocAccount>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GethConfig {
    #[serde(default, rename = "chainId")]
    pub(crate) chain_id: Option<U64Like>,
    #[serde(default, rename = "homesteadBlock")]
    pub(crate) homestead_block: Option<U64Like>,
    #[serde(default, rename = "eip150Block")]
    pub(crate) eip150_block: Option<U64Like>,
    #[serde(default, rename = "eip155Block")]
    pub(crate) eip155_block: Option<U64Like>,
    #[serde(default, rename = "eip158Block")]
    pub(crate) eip158_block: Option<U64Like>,
    #[serde(default, rename = "byzantiumBlock")]
    pub(crate) byzantium_block: Option<U64Like>,
    #[serde(default, rename = "constantinopleBlock")]
    pub(crate) constantinople_block: Option<U64Like>,
    #[serde(default, rename = "petersburgBlock")]
    pub(crate) petersburg_block: Option<U64Like>,
    #[serde(default, rename = "istanbulBlock")]
    pub(crate) istanbul_block: Option<U64Like>,
    #[serde(default, rename = "muirGlacierBlock")]
    pub(crate) muir_glacier_block: Option<U64Like>,
    #[serde(default, rename = "berlinBlock")]
    pub(crate) berlin_block: Option<U64Like>,
    #[serde(default, rename = "londonBlock")]
    pub(crate) london_block: Option<U64Like>,
    #[serde(default, rename = "arrowGlacierBlock")]
    pub(crate) arrow_glacier_block: Option<U64Like>,
    #[serde(default, rename = "grayGlacierBlock")]
    pub(crate) gray_glacier_block: Option<U64Like>,
    #[serde(default, rename = "mergeNetsplitBlock")]
    pub(crate) merge_netsplit_block: Option<U64Like>,
    #[serde(default, rename = "shanghaiTime")]
    pub(crate) shanghai_time: Option<U64Like>,
    #[serde(default, rename = "cancunTime")]
    pub(crate) cancun_time: Option<U64Like>,
    #[serde(default, rename = "pragueTime")]
    pub(crate) prague_time: Option<U64Like>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GethAllocAccount {
    #[serde(default)]
    pub(crate) code: Option<String>,
    #[serde(default)]
    pub(crate) nonce: Option<U64Like>,
    #[serde(default)]
    pub(crate) storage: Option<BTreeMap<String, U256Like>>,
    #[serde(default)]
    pub(crate) balance: Option<U256Like>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum U64Like {
    Number(u64),
    String(String),
}

impl U64Like {
    pub(crate) fn to_u64(&self, field_name: &str) -> Result<u64> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::String(raw) => decode_u64_literal(raw, field_name),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum U256Like {
    Number(u64),
    String(String),
}

impl U256Like {
    pub(crate) fn to_u256(&self, field_name: &str) -> Result<U256> {
        match self {
            Self::Number(value) => Ok(U256::from(*value)),
            Self::String(raw) => decode_u256_literal(raw, field_name),
        }
    }
}

fn decode_u64_literal(raw: &str, field_name: &str) -> Result<u64> {
    let value = raw.trim();
    if let Some(stripped) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if stripped.is_empty() {
            return Ok(0);
        }
        return u64::from_str_radix(stripped, 16)
            .with_context(|| format!("Invalid hex integer for {field_name}: {raw}"));
    }
    value
        .parse::<u64>()
        .with_context(|| format!("Invalid integer for {field_name}: {raw}"))
}

pub(crate) fn decode_u256_literal(raw: &str, field_name: &str) -> Result<U256> {
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

pub(crate) fn value_as_optional_u64(
    value: Option<&Value>,
    field_name: &str,
) -> Result<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let decoded: U64Like = serde_json::from_value(value.clone())
        .with_context(|| format!("Invalid integer for {field_name}: {value}"))?;
    Ok(Some(decoded.to_u64(field_name)?))
}

pub(crate) fn value_as_u256(value: &Value, field_name: &str) -> Result<U256> {
    let decoded: U256Like = serde_json::from_value(value.clone())
        .with_context(|| format!("Invalid integer for {field_name}: {value}"))?;
    decoded.to_u256(field_name)
}

pub(crate) fn decode_hex_address(raw: &str) -> Result<Address> {
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

pub(crate) fn object_field_or_insert<'a>(
    parent: &'a mut Value,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn u64_like_accepts_hex_and_decimal() {
        let hex: U64Like = serde_json::from_value(json!("0x2a")).unwrap();
        let dec: U64Like = serde_json::from_value(json!("42")).unwrap();
        assert_eq!(hex.to_u64("field").unwrap(), 42);
        assert_eq!(dec.to_u64("field").unwrap(), 42);
    }

    #[test]
    fn u256_like_accepts_empty_hex() {
        let value: U256Like = serde_json::from_value(json!("0x")).unwrap();
        assert_eq!(value.to_u256("field").unwrap(), U256::ZERO);
    }

    #[test]
    fn decode_hex_address_rejects_bad_length() {
        assert!(decode_hex_address("0x1234").is_err());
    }

    #[test]
    fn value_as_optional_u64_accepts_null() {
        assert_eq!(
            value_as_optional_u64(Some(&Value::Null), "field").unwrap(),
            None
        );
    }
}
