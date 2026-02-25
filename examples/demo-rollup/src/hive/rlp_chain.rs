use std::fs;
use std::path::Path;

use alloy_rlp::{Decodable, Header, PayloadView};
use anyhow::{anyhow, bail, Context, Result};

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

pub(crate) fn load_chain_timestamps(chain_rlp_path: &Path) -> Result<Vec<(u64, u64)>> {
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

pub(crate) fn activation_block_for_timestamp(
    chain_timestamps: &[(u64, u64)],
    timestamp: u64,
) -> Option<u64> {
    chain_timestamps
        .iter()
        .find_map(|(block_number, block_ts)| (*block_ts >= timestamp).then_some(*block_number))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_rlp::Encodable;

    use super::*;

    #[test]
    fn activation_block_uses_first_timestamp_greater_or_equal() {
        let ts = vec![(10, 100), (11, 200), (12, 300)];
        assert_eq!(activation_block_for_timestamp(&ts, 50), Some(10));
        assert_eq!(activation_block_for_timestamp(&ts, 200), Some(11));
        assert_eq!(activation_block_for_timestamp(&ts, 301), None);
    }

    #[test]
    fn decode_rlp_u64_roundtrip() {
        let mut encoded = Vec::new();
        42u64.encode(&mut encoded);
        assert_eq!(decode_rlp_u64(&encoded, "value").unwrap(), 42);
    }

    #[test]
    fn decode_rlp_u64_rejects_trailing_bytes() {
        let mut encoded = Vec::new();
        7u64.encode(&mut encoded);
        encoded.push(0x00);
        assert!(decode_rlp_u64(&encoded, "value").is_err());
    }

    #[test]
    fn load_chain_timestamps_rejects_malformed_block() {
        let path = std::env::temp_dir().join(format!(
            "sov-hive-genesis-adapter-invalid-chain-{}.rlp",
            std::process::id()
        ));
        fs::write(&path, [0xc0]).unwrap();

        let result = load_chain_timestamps(&path);
        let _ = fs::remove_file(&path);

        assert!(result.is_err());
    }
}
