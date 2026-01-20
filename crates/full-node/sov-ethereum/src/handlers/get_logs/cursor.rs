use hex::{decode, encode};
use jsonrpsee::types::ErrorObjectOwned;

use crate::rpc_invalid_params;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Cursor indicating where to start processing.
pub struct Cursor {
    /// Starting block height for this cursor.
    pub block_height: u64,
    /// Absolute index of the first transaction to process.
    pub tx_index_absolute: u64,
    /// Index of the first log within that transaction to process.
    pub log_index_in_tx: u32,
}

impl Cursor {
    /// Packs `Self` into a 40-character hex string (20 bytes total).
    /// Layout (big-endian): [block_height:8][tx_index_absolute:8][log_index_in_tx:4]
    pub fn pack(&self) -> String {
        let mut bytes = [0u8; 20];
        bytes[0..8].copy_from_slice(&self.block_height.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.tx_index_absolute.to_be_bytes());
        bytes[16..20].copy_from_slice(&self.log_index_in_tx.to_be_bytes());

        encode(bytes)
    }

    /// Unpacks a 40-character (or "0x"-prefixed) hex string into `Self`.
    pub fn unpack(hex_str: &str) -> Result<Self, ErrorObjectOwned> {
        let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let bytes = decode(s).map_err(|_| rpc_invalid_params("Invalid hex string"))?;

        if bytes.len() != 20 {
            let msg = format!(
                "Invalid decoded length expected 20 bytes, got {}",
                bytes.len()
            );
            return Err(rpc_invalid_params(msg));
        }

        let block_height = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
        let tx_index_absolute = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        let log_index_in_tx = u32::from_be_bytes(bytes[16..20].try_into().unwrap());

        Ok(Self {
            block_height,
            tx_index_absolute,
            log_index_in_tx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Cursor;
    use hex::encode;

    #[test]
    fn pack_known_vector() {
        let c = Cursor {
            block_height: 12345,
            tx_index_absolute: 67890,
            log_index_in_tx: 42,
        };

        let hex = c.pack();
        assert_eq!(hex, "000000000000303900000000000109320000002a");
        assert_eq!(hex.len(), 40);
    }

    #[test]
    fn unpack_known_vector() {
        let hex = "000000000000303900000000000109320000002a";
        let c = Cursor::unpack(hex).unwrap();
        assert_eq!(c.block_height, 12345);
        assert_eq!(c.tx_index_absolute, 67890);
        assert_eq!(c.log_index_in_tx, 42);
    }

    #[test]
    fn unpack_accepts_0x_prefix() {
        let hex = "0x000000000000303900000000000109320000002a";
        let c = Cursor::unpack(hex).unwrap();
        assert_eq!(c.block_height, 12345);
        assert_eq!(c.tx_index_absolute, 67890);
        assert_eq!(c.log_index_in_tx, 42);
    }

    #[test]
    fn roundtrip_various_values() {
        let cases = [
            Cursor {
                block_height: 0,
                tx_index_absolute: 0,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: 1,
                tx_index_absolute: 2,
                log_index_in_tx: 3,
            },
            Cursor {
                block_height: u64::MAX,
                tx_index_absolute: 0,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: 0,
                tx_index_absolute: u64::MAX,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: 0,
                tx_index_absolute: 0,
                log_index_in_tx: u32::MAX,
            },
            Cursor {
                block_height: u64::MAX,
                tx_index_absolute: u64::MAX,
                log_index_in_tx: u32::MAX,
            },
            Cursor {
                block_height: 42,
                tx_index_absolute: 1_000_000,
                log_index_in_tx: 999,
            },
        ];

        for c in cases {
            let hex = c.pack();
            assert_eq!(hex.len(), 40, "hex length must be fixed 40");
            let decoded = Cursor::unpack(&hex).unwrap();
            assert_eq!(decoded, c);
        }
    }

    #[test]
    fn pack_endianness_layout() {
        // Manually construct the expected 20 bytes: BE([bh:8][tx:8][log:4])
        let c = Cursor {
            block_height: 0x1122_3344_5566_7788,
            tx_index_absolute: 0x99aa_bbcc_ddee_ff00,
            log_index_in_tx: 0x1234_5678,
        };

        // Expected bytes in big-endian layout
        let mut expected = Vec::new();
        expected.extend_from_slice(&c.block_height.to_be_bytes());
        expected.extend_from_slice(&c.tx_index_absolute.to_be_bytes());
        expected.extend_from_slice(&c.log_index_in_tx.to_be_bytes());

        let hex = c.pack();
        assert_eq!(hex, encode(expected));
    }

    #[test]
    fn pack_all_maxes_is_all_fs() {
        let c = Cursor {
            block_height: u64::MAX,
            tx_index_absolute: u64::MAX,
            log_index_in_tx: u32::MAX,
        };
        let hex = c.pack();
        assert_eq!(hex, "ffffffffffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn unpack_invalid_char() {
        // Not valid hex
        assert!(Cursor::unpack("zz0000000000303900000000000109320000002a").is_err());
    }

    #[test]
    fn unpack_wrong_length_panics() {
        // 38 hex chars -> 19 bytes after decode -> triggers assert on length
        assert!(Cursor::unpack("00000000000030390000000000010932000000").is_err());
    }

    #[test]
    fn leading_zeros_preserved_on_pack() {
        let c = Cursor {
            block_height: 1, // lots of leading zeros
            tx_index_absolute: 0,
            log_index_in_tx: 0,
        };
        let hex = c.pack();
        // First 8 bytes are block_height; only the last byte is 0x01
        assert_eq!(&hex[0..16], "0000000000000001");
        assert_eq!(hex.len(), 40);
    }
}
