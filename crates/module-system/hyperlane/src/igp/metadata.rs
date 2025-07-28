use std::io::{Error, ErrorKind, Result};

use sov_bank::Amount;

/// Metadata with fields for IGP, we skip first fields since we don't need them.
///
/// See <https://docs.hyperlane.xyz/docs/reference/libraries/hookmetadata>
///
/// (0:2) variant
/// (2:34) msg.value
/// (34:66) Gas limit for message (IGP)
/// (66:86) Refund address for message (IGP)
/// (86:) Custom metadata
pub struct IGPMetadata {
    /// Gas limit.
    ///
    /// NOTE: in Hyperlane they use u256 but we only write/read last 16 bytes since sovereign sdk
    /// uses u128 for amount.
    pub gas_limit: Amount,
}

impl IGPMetadata {
    pub(crate) fn deserialize(buf: &[u8]) -> Result<Self> {
        const MIN_LENGTH: usize = 66;
        const GAS_LIMIT_OFFSET: usize = 34;

        if buf.len() < MIN_LENGTH {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Expected at least {MIN_LENGTH} bytes for IGPMetadata, got {}",
                    buf.len()
                ),
            ));
        }

        // Extract gas_limit from position 34-66
        // We only need the last 16 bytes for u128
        // So check if first 16 are zeroes
        let first_half = &buf[GAS_LIMIT_OFFSET..GAS_LIMIT_OFFSET + 16];
        if !first_half.iter().all(|&b| b == 0) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Gas limit exceeds u128 maximum",
            ));
        }

        let gas_limit = match buf[GAS_LIMIT_OFFSET + 16..GAS_LIMIT_OFFSET + 32].try_into() {
            Ok(bytes) => bytes,
            Err(_) => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "Failed to convert gas limit bytes to array",
                ))
            }
        };
        let gas_limit = u128::from_be_bytes(gas_limit);

        if gas_limit == 0 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Gas limit is not set, all bytes are 0",
            ));
        }

        Ok(Self {
            gas_limit: Amount(gas_limit),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use sov_bank::Amount;

    use super::IGPMetadata;

    #[test]
    fn igp_metadata_deserialize() {
        let metadata = IGPMetadata {
            gas_limit: Amount(14235043),
        };
        let mut buf = vec![0_u8; 86];
        let gas_limit_bytes = metadata.gas_limit.0.to_be_bytes();
        buf[34 + 16..34 + 32].copy_from_slice(&gas_limit_bytes);
        let decoded = IGPMetadata::deserialize(&buf).expect("should deserialize");
        assert_eq!(decoded.gas_limit, metadata.gas_limit);
    }

    #[test]
    fn igp_metadata_deserialize_max_u128() {
        let metadata = IGPMetadata {
            gas_limit: Amount(u128::MAX),
        };
        let mut buf = vec![0_u8; 86];
        let gas_limit_bytes = metadata.gas_limit.0.to_be_bytes();
        buf[34 + 16..34 + 32].copy_from_slice(&gas_limit_bytes);
        let decoded = IGPMetadata::deserialize(&buf).expect("should deserialize");
        assert_eq!(decoded.gas_limit, metadata.gas_limit);
    }

    #[test]
    fn igp_metadata_deserialize_exceeds_u128() {
        // This simulates a U256 value that's too large for u128
        let mut buf = vec![0_u8; 86];
        // Set the first byte of the gas limit to non-zero
        buf[34] = 1;
        // Fill the rest with valid data
        buf[34 + 16..34 + 32].copy_from_slice(&u128::MAX.to_be_bytes());

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(e.to_string(), "Gas limit exceeds u128 maximum");
        }
    }

    #[test]
    fn igp_metadata_deserialize_zero_gas_limit() {
        let buf = vec![0_u8; 86];

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(e.to_string(), "Gas limit is not set, all bytes are 0");
        }
    }

    #[test]
    fn igp_metadata_deserialize_buffer_too_small() {
        let buf = vec![0_u8; 65]; // MIN_LENGTH is 66

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert!(e.to_string().contains("Expected at least 66 bytes"));
        }
    }
}
