use alloy_primitives::{hex, Address, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

pub(crate) fn parse_u64_literal(raw: &str, field_name: &str) -> Result<u64> {
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

pub(crate) fn parse_u256_literal(raw: &str, field_name: &str) -> Result<U256> {
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

pub(crate) fn parse_u64_value(value: &Value, field_name: &str) -> Result<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow!("Invalid integer for {field_name}: {value}")),
        Value::String(s) => parse_u64_literal(s, field_name),
        _ => bail!("Invalid integer for {field_name}: {value}"),
    }
}

pub(crate) fn parse_optional_u64(value: Option<&Value>, field_name: &str) -> Result<Option<u64>> {
    match value {
        Some(Value::Null) | None => Ok(None),
        Some(v) => Ok(Some(parse_u64_value(v, field_name)?)),
    }
}

pub(crate) fn parse_u256_value(value: &Value, field_name: &str) -> Result<U256> {
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

pub(crate) fn normalize_hex_address(raw: &str) -> Result<Address> {
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

pub(crate) fn normalize_hex_data(raw: &str, field_name: &str) -> Result<String> {
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

pub(crate) fn to_hex_u256(value: U256) -> String {
    format!("0x{value:x}")
}

pub(crate) fn parse_storage_map(
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

pub(crate) fn ensure_object_field<'a>(
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
    use super::*;

    #[test]
    fn parse_u64_literal_accepts_hex_and_decimal() {
        assert_eq!(parse_u64_literal("0x2a", "field").unwrap(), 42);
        assert_eq!(parse_u64_literal("42", "field").unwrap(), 42);
    }

    #[test]
    fn parse_u256_literal_accepts_empty_hex() {
        assert_eq!(parse_u256_literal("0x", "field").unwrap(), U256::ZERO);
    }

    #[test]
    fn normalize_hex_data_pads_odd_length() {
        assert_eq!(
            normalize_hex_data("0xabc", "field").unwrap(),
            "0x0abc".to_string()
        );
    }

    #[test]
    fn normalize_hex_address_rejects_bad_length() {
        assert!(normalize_hex_address("0x1234").is_err());
    }

    #[test]
    fn parse_storage_map_normalizes_slot_and_value() {
        let mut input = Map::new();
        input.insert("0x1".to_string(), Value::String("0x2".to_string()));
        let out = parse_storage_map(&input, "acc").unwrap();
        assert_eq!(
            out.get("0x1"),
            Some(&Value::String("0x2".to_string())),
            "slot and value should be normalized to quantity-style hex"
        );
    }
}
