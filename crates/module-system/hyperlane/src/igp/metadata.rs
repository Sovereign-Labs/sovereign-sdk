use std::io::{Error, ErrorKind, Result};

use sov_bank::Amount;

/// IGP metadata following Hyperlane's `StandardHookMetadata` layout. We only
/// retain the gas limit; `variant` is validated on deserialization and the other
/// fields are advisory and ignored.
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
    /// NOTE: Hyperlane encodes this as a u256, but Sovereign SDK uses u128 for
    /// amounts, so values exceeding `u128::MAX` are rejected during deserialization.
    pub gas_limit: Amount,
}

/// Reads the next `N` bytes from `buf`, advancing it past them.
fn read_field<const N: usize>(buf: &mut &[u8]) -> Result<[u8; N]> {
    let (field, rest) = buf
        .split_at_checked(N)
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "IGPMetadata is too short"))?;
    *buf = rest;
    Ok(field.try_into().expect("slice length checked above"))
}

impl IGPMetadata {
    /// The only metadata format version we support, matching Hyperlane's
    /// `StandardHookMetadata.VARIANT`.
    const EXPECTED_VARIANT: u16 = 1;

    pub(crate) fn deserialize(buf: &[u8]) -> Result<Self> {
        let mut cursor = buf;

        // variant (0:2): reject unknown versions so a future format isn't misparsed.
        let variant = u16::from_be_bytes(read_field(&mut cursor)?);
        if variant != Self::EXPECTED_VARIANT {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Unsupported IGPMetadata variant: expected {}, got {variant}",
                    Self::EXPECTED_VARIANT
                ),
            ));
        }

        // msg.value (2:34): advisory, the gas paymaster doesn't act on it.
        read_field::<32>(&mut cursor)?;

        // gas limit (34:66): Hyperlane uses a u256 but our amounts are u128, so
        // anything beyond u128::MAX is rejected.
        let gas_limit = ruint::Uint::<256, 4>::from_be_bytes(read_field::<32>(&mut cursor)?);
        let gas_limit: u128 = gas_limit
            .try_into()
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Gas limit exceeds u128 maximum"))?;
        if gas_limit == 0 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Gas limit is not set, all bytes are 0",
            ));
        }

        // refund address (66:86) and trailing custom metadata are advisory; we
        // charge exactly the required gas and never refund, so they're ignored.

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

    /// Builds a valid `StandardHookMetadata` buffer (variant 1, zero msg.value,
    /// zero refund address) with the given gas limit.
    fn valid_buf(gas_limit: u128) -> Vec<u8> {
        let mut buf = vec![0_u8; 86];
        // variant = 1
        buf[0..2].copy_from_slice(&1u16.to_be_bytes());
        // gas limit lives in the lower 16 bytes of the (34:66) field
        buf[34 + 16..34 + 32].copy_from_slice(&gas_limit.to_be_bytes());
        buf
    }

    #[test]
    fn igp_metadata_deserialize() {
        let buf = valid_buf(14235043);
        let decoded = IGPMetadata::deserialize(&buf).expect("should deserialize");
        assert_eq!(decoded.gas_limit, Amount(14235043));
    }

    #[test]
    fn igp_metadata_deserialize_max_u128() {
        let buf = valid_buf(u128::MAX);
        let decoded = IGPMetadata::deserialize(&buf).expect("should deserialize");
        assert_eq!(decoded.gas_limit, Amount(u128::MAX));
    }

    #[test]
    fn igp_metadata_deserialize_exceeds_u128() {
        // This simulates a U256 value that's too large for u128
        let mut buf = valid_buf(u128::MAX);
        // Set the first byte of the gas limit to non-zero
        buf[34] = 1;

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(e.to_string(), "Gas limit exceeds u128 maximum");
        }
    }

    #[test]
    fn igp_metadata_deserialize_zero_gas_limit() {
        let buf = valid_buf(0);

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(e.to_string(), "Gas limit is not set, all bytes are 0");
        }
    }

    #[test]
    fn igp_metadata_deserialize_buffer_too_small() {
        // Truncated partway through the gas limit field.
        let buf = valid_buf(14235043);
        let result = IGPMetadata::deserialize(&buf[..65]);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(e.to_string(), "IGPMetadata is too short");
        }
    }

    #[test]
    fn igp_metadata_deserialize_unsupported_variant() {
        let mut buf = valid_buf(14235043);
        // variant = 2 is not a format we understand
        buf[0..2].copy_from_slice(&2u16.to_be_bytes());

        let result = IGPMetadata::deserialize(&buf);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.kind(), ErrorKind::InvalidData);
            assert_eq!(
                e.to_string(),
                "Unsupported IGPMetadata variant: expected 1, got 2"
            );
        }
    }

    #[test]
    fn igp_metadata_deserialize_ignores_nonzero_msg_value() {
        let mut buf = valid_buf(14235043);
        // A non-zero msg.value (2:34) must not affect parsing.
        buf[33] = 0x42;

        let decoded = IGPMetadata::deserialize(&buf).expect("non-zero msg.value should be ignored");
        assert_eq!(decoded.gas_limit, Amount(14235043));
    }

    #[test]
    fn igp_metadata_deserialize_ignores_nonzero_refund_address() {
        let mut buf = valid_buf(14235043);
        // A non-zero refund address (66:86) is the common case and must be accepted.
        buf[66] = 0xab;

        let decoded =
            IGPMetadata::deserialize(&buf).expect("non-zero refund address should be ignored");
        assert_eq!(decoded.gas_limit, Amount(14235043));
    }

    #[test]
    fn igp_metadata_deserialize_refund_address_omitted() {
        // Exactly 66 bytes: the refund address field is omitted, which is fine.
        let buf = valid_buf(14235043);
        let decoded = IGPMetadata::deserialize(&buf[..66])
            .expect("should deserialize without refund address");
        assert_eq!(decoded.gas_limit, Amount(14235043));
    }
}
