use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
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

    if let Some(value) = value.as_u64() {
        return Ok(Some(value));
    }

    if let Some(value) = value.as_str() {
        return Ok(Some(decode_u64_literal(value, field_name)?));
    }

    bail!("Invalid integer for {field_name}: {value}")
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
    fn value_as_optional_u64_accepts_hex_and_decimal_strings() {
        assert_eq!(
            value_as_optional_u64(Some(&json!("0x2a")), "field").unwrap(),
            Some(42)
        );
        assert_eq!(
            value_as_optional_u64(Some(&json!("42")), "field").unwrap(),
            Some(42)
        );
    }

    #[test]
    fn value_as_optional_u64_accepts_number() {
        assert_eq!(
            value_as_optional_u64(Some(&json!(42)), "field").unwrap(),
            Some(42)
        );
    }

    #[test]
    fn value_as_optional_u64_accepts_null() {
        assert_eq!(
            value_as_optional_u64(Some(&Value::Null), "field").unwrap(),
            None
        );
    }
}
