use std::fs;
use std::path::Path;

use alloy::consensus::Header as ConsensusHeader;
use alloy_rlp::{Decodable, Header, PayloadView};
use anyhow::{bail, Context, Result};

pub(crate) fn load_chain_timestamps(chain_rlp_path: &Path) -> Result<Vec<(u64, u64)>> {
    let data = fs::read(chain_rlp_path)
        .with_context(|| format!("Failed to read {}", chain_rlp_path.display()))?;
    let mut out = Vec::new();
    let mut cursor = data.as_slice();

    while !cursor.is_empty() {
        let payload = Header::decode_raw(&mut cursor).with_context(|| {
            format!(
                "Failed to decode RLP payload for chain.rlp block from {}",
                chain_rlp_path.display()
            )
        })?;
        let block_fields = match payload {
            PayloadView::List(items) => items,
            PayloadView::String(_) => bail!("Expected RLP list for chain.rlp block"),
        };
        if block_fields.is_empty() {
            bail!("Malformed block in chain.rlp");
        }

        let mut header_raw = block_fields[0];
        let header = ConsensusHeader::decode(&mut header_raw)
            .context("Failed to decode chain.rlp block header")?;
        if !header_raw.is_empty() {
            bail!("Malformed chain.rlp block header: trailing bytes");
        }
        out.push((header.number, header.timestamp));
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

    use alloy::consensus::Header as ConsensusHeader;
    use alloy_rlp::{Encodable, Header as RlpHeader};

    use super::*;

    fn encode_block_item(header: &ConsensusHeader) -> Vec<u8> {
        let mut header_bytes = Vec::new();
        header.encode(&mut header_bytes);

        let mut payload = Vec::new();
        payload.extend_from_slice(&header_bytes);
        payload.push(0xc0); // transactions = empty list
        payload.push(0xc0); // ommers = empty list

        let mut out = Vec::new();
        RlpHeader {
            list: true,
            payload_length: payload.len(),
        }
        .encode(&mut out);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn activation_block_uses_first_timestamp_greater_or_equal() {
        let ts = vec![(10, 100), (11, 200), (12, 300)];
        assert_eq!(activation_block_for_timestamp(&ts, 50), Some(10));
        assert_eq!(activation_block_for_timestamp(&ts, 200), Some(11));
        assert_eq!(activation_block_for_timestamp(&ts, 301), None);
    }

    #[test]
    fn load_chain_timestamps_decodes_via_alloy_header() {
        let path = std::env::temp_dir().join(format!(
            "sov-hive-genesis-adapter-valid-chain-{}.rlp",
            std::process::id()
        ));

        let first = ConsensusHeader {
            number: 7,
            timestamp: 100,
            ..Default::default()
        };

        let second = ConsensusHeader {
            number: 8,
            timestamp: 200,
            base_fee_per_gas: Some(1),
            ..Default::default()
        };

        let mut chain = Vec::new();
        chain.extend_from_slice(&encode_block_item(&first));
        chain.extend_from_slice(&encode_block_item(&second));
        fs::write(&path, chain).unwrap();

        let result = load_chain_timestamps(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(result, vec![(7, 100), (8, 200)]);
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
