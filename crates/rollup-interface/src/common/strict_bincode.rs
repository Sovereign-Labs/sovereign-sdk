//! Strict bincode deserialization helper.
use bincode::Options as _;
use serde::de::DeserializeOwned;

/// Deserialize `bytes` into `T`, rejecting any trailing bytes.
///
/// Drop-in replacement for [`bincode::deserialize`] for cases where the
/// payload length is supposed to match the encoded value exactly.
pub fn strict_bincode_deserialize<T: DeserializeOwned>(bytes: &[u8]) -> bincode::Result<T> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(bytes.len() as u64)
        .reject_trailing_bytes()
        .deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_matches_default_bincode() {
        let value: (u32, String) = (0x12345678, "hello".to_string());
        let bytes = bincode::serialize(&value).unwrap();
        let decoded: (u32, String) = strict_bincode_deserialize(&bytes).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn rejects_trailing_bytes() {
        let bytes = bincode::serialize(&0x12345678u32).unwrap();
        let mut padded = bytes.clone();
        padded.push(0xff);

        let lax: u32 = bincode::deserialize(&padded).unwrap();
        assert_eq!(
            lax, 0x12345678,
            "sanity check: stock bincode must accept trailing bytes",
        );

        let strict: bincode::Result<u32> = strict_bincode_deserialize(&padded);
        assert!(
            strict.is_err(),
            "strict_bincode_deserialize must reject trailing bytes",
        );
    }

    #[test]
    fn rejects_truncated_input() {
        let bytes = bincode::serialize(&(0x12345678u32, 0xabcdu16)).unwrap();
        let truncated = &bytes[..bytes.len() - 1];

        let strict: bincode::Result<(u32, u16)> = strict_bincode_deserialize(truncated);
        assert!(
            strict.is_err(),
            "strict_bincode_deserialize must reject truncated input",
        );
    }
}
