//! Local Ethereum receipt type, replacing `reth_primitives::Receipt` to avoid the reth dependency.

use alloy_consensus::{
    Eip2718EncodableReceipt, Eip658Value, RlpEncodableReceipt, TxReceipt, TxType, Typed2718,
};
use alloy_eips::Encodable2718;
use alloy_primitives::{Bloom, Log};
use alloy_rlp::{BufMut, Encodable, Header};

/// Ethereum transaction receipt.
#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct EthReceipt {
    /// Receipt type.
    pub tx_type: TxType,
    /// If transaction is executed successfully.
    pub success: bool,
    /// Cumulative gas used in the block after this transaction was executed.
    pub cumulative_gas_used: u64,
    /// Logs emitted by this transaction.
    pub logs: Vec<Log>,
}

impl EthReceipt {
    /// Returns length of RLP-encoded receipt fields with the given [`Bloom`] without an RLP header.
    fn rlp_encoded_fields_length(&self, bloom: &Bloom) -> usize {
        self.success.length()
            + self.cumulative_gas_used.length()
            + bloom.length()
            + self.logs.length()
    }

    /// RLP-encodes receipt fields with the given [`Bloom`] without an RLP header.
    fn rlp_encode_fields(&self, bloom: &Bloom, out: &mut dyn BufMut) {
        self.success.encode(out);
        self.cumulative_gas_used.encode(out);
        bloom.encode(out);
        self.logs.encode(out);
    }

    /// Returns RLP header for inner encoding (with bloom).
    fn rlp_header_inner(&self, bloom: &Bloom) -> Header {
        Header {
            list: true,
            payload_length: self.rlp_encoded_fields_length(bloom),
        }
    }

    /// Returns length of RLP-encoded receipt fields without bloom and without an RLP header.
    fn rlp_encoded_fields_length_without_bloom(&self) -> usize {
        self.success.length() + self.cumulative_gas_used.length() + self.logs.length()
    }

    /// RLP-encodes receipt fields without bloom and without an RLP header.
    fn rlp_encode_fields_without_bloom(&self, out: &mut dyn BufMut) {
        self.success.encode(out);
        self.cumulative_gas_used.encode(out);
        self.logs.encode(out);
    }

    /// Returns RLP header for inner encoding (without bloom).
    fn rlp_header_inner_without_bloom(&self) -> Header {
        Header {
            list: true,
            payload_length: self.rlp_encoded_fields_length_without_bloom(),
        }
    }
}

impl TxReceipt for EthReceipt {
    type Log = Log;

    fn status_or_post_state(&self) -> Eip658Value {
        self.success.into()
    }

    fn status(&self) -> bool {
        self.success
    }

    fn bloom(&self) -> Bloom {
        alloy_primitives::logs_bloom(self.logs.iter())
    }

    fn cumulative_gas_used(&self) -> u64 {
        self.cumulative_gas_used
    }

    fn logs(&self) -> &[Log] {
        &self.logs
    }

    fn into_logs(self) -> Vec<Log> {
        self.logs
    }
}

impl Typed2718 for EthReceipt {
    fn ty(&self) -> u8 {
        self.tx_type.ty()
    }
}

impl Eip2718EncodableReceipt for EthReceipt {
    fn eip2718_encoded_length_with_bloom(&self, bloom: &Bloom) -> usize {
        !self.tx_type.is_legacy() as usize + self.rlp_header_inner(bloom).length_with_payload()
    }

    fn eip2718_encode_with_bloom(&self, bloom: &Bloom, out: &mut dyn BufMut) {
        if !self.tx_type.is_legacy() {
            out.put_u8(self.tx_type.ty());
        }
        self.rlp_header_inner(bloom).encode(out);
        self.rlp_encode_fields(bloom, out);
    }
}

impl RlpEncodableReceipt for EthReceipt {
    fn rlp_encoded_length_with_bloom(&self, bloom: &Bloom) -> usize {
        let mut len = self.eip2718_encoded_length_with_bloom(bloom);
        if !self.tx_type.is_legacy() {
            len += Header {
                list: false,
                payload_length: self.eip2718_encoded_length_with_bloom(bloom),
            }
            .length();
        }
        len
    }

    fn rlp_encode_with_bloom(&self, bloom: &Bloom, out: &mut dyn BufMut) {
        if !self.tx_type.is_legacy() {
            Header {
                list: false,
                payload_length: self.eip2718_encoded_length_with_bloom(bloom),
            }
            .encode(out);
        }
        self.eip2718_encode_with_bloom(bloom, out);
    }
}

impl Encodable2718 for EthReceipt {
    fn encode_2718_len(&self) -> usize {
        (!self.tx_type.is_legacy() as usize)
            + self.rlp_header_inner_without_bloom().length_with_payload()
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        if !self.tx_type.is_legacy() {
            out.put_u8(self.tx_type.ty());
        }
        self.rlp_header_inner_without_bloom().encode(out);
        self.rlp_encode_fields_without_bloom(out);
    }
}

/// Bincode-compatible serialization for [`EthReceipt`], matching the format
/// previously provided by `reth_ethereum_primitives::serde_bincode_compat::Receipt`.
#[allow(clippy::owned_cow)]
pub mod serde_bincode_compat {
    use alloc::borrow::Cow;
    use alloy_consensus::TxType;
    use alloy_primitives::{Log, U8};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    extern crate alloc;

    /// Bincode-compatible receipt representation.
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Receipt<'a> {
        #[serde(deserialize_with = "deserialize_txtype")]
        tx_type: TxType,
        success: bool,
        cumulative_gas_used: u64,
        logs: Cow<'a, Vec<Log>>,
    }

    fn deserialize_txtype<'de, D>(deserializer: D) -> Result<TxType, D::Error>
    where
        D: Deserializer<'de>,
    {
        U8::deserialize(deserializer)?
            .to::<u8>()
            .try_into()
            .map_err(serde::de::Error::custom)
    }

    impl<'a> From<&'a super::EthReceipt> for Receipt<'a> {
        fn from(value: &'a super::EthReceipt) -> Self {
            Self {
                tx_type: value.tx_type,
                success: value.success,
                cumulative_gas_used: value.cumulative_gas_used,
                logs: Cow::Borrowed(&value.logs),
            }
        }
    }

    impl From<Receipt<'_>> for super::EthReceipt {
        fn from(value: Receipt<'_>) -> Self {
            Self {
                tx_type: value.tx_type,
                success: value.success,
                cumulative_gas_used: value.cumulative_gas_used,
                logs: value.logs.into_owned(),
            }
        }
    }

    impl SerializeAs<super::EthReceipt> for Receipt<'_> {
        fn serialize_as<S>(source: &super::EthReceipt, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Receipt::from(source).serialize(serializer)
        }
    }

    impl<'de> DeserializeAs<'de, super::EthReceipt> for Receipt<'de> {
        fn deserialize_as<D>(deserializer: D) -> Result<super::EthReceipt, D::Error>
        where
            D: Deserializer<'de>,
        {
            Receipt::deserialize(deserializer).map(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxType;
    use alloy_primitives::{Address, Bytes, Log, B256};
    use serde_with::serde_as;

    /// Wrapper that serializes [`EthReceipt`] through our local bincode-compat adapter.
    #[serde_as]
    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct LocalWrapper {
        #[serde_as(as = "serde_bincode_compat::Receipt<'_>")]
        receipt: EthReceipt,
    }

    /// Wrapper that serializes [`reth_ethereum_primitives::Receipt`] through reth's adapter.
    #[serde_as]
    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct RethWrapper {
        #[serde_as(as = "reth_ethereum_primitives::serde_bincode_compat::Receipt<'_>")]
        receipt: reth_ethereum_primitives::Receipt,
    }

    fn make_reth_receipt(r: &EthReceipt) -> reth_ethereum_primitives::Receipt {
        reth_ethereum_primitives::Receipt {
            tx_type: r.tx_type,
            success: r.success,
            cumulative_gas_used: r.cumulative_gas_used,
            logs: r.logs.clone(),
        }
    }

    /// Test cases with enough variability to catch serialization mismatches.
    fn test_receipts() -> Vec<(&'static str, EthReceipt)> {
        vec![
            (
                "legacy_success_no_logs",
                EthReceipt {
                    tx_type: TxType::Legacy,
                    success: true,
                    cumulative_gas_used: 21000,
                    logs: vec![],
                },
            ),
            (
                "eip1559_failure_no_logs",
                EthReceipt {
                    tx_type: TxType::Eip1559,
                    success: false,
                    cumulative_gas_used: 0,
                    logs: vec![],
                },
            ),
            (
                "eip2930_success_with_log",
                EthReceipt {
                    tx_type: TxType::Eip2930,
                    success: true,
                    cumulative_gas_used: 50000,
                    logs: vec![Log::new_unchecked(
                        Address::with_last_byte(0xAB),
                        vec![B256::with_last_byte(0x01)],
                        Bytes::from_static(&[0xDE, 0xAD]),
                    )],
                },
            ),
            (
                "eip1559_success_multiple_logs",
                EthReceipt {
                    tx_type: TxType::Eip1559,
                    success: true,
                    cumulative_gas_used: 1_000_000,
                    logs: vec![
                        Log::new_unchecked(
                            Address::with_last_byte(0x01),
                            vec![B256::with_last_byte(0xAA), B256::with_last_byte(0xBB)],
                            Bytes::from_static(&[1, 2, 3, 4, 5]),
                        ),
                        Log::new_unchecked(Address::ZERO, vec![], Bytes::new()),
                    ],
                },
            ),
            (
                "eip7702_high_gas",
                EthReceipt {
                    tx_type: TxType::Eip7702,
                    success: true,
                    cumulative_gas_used: u64::MAX,
                    logs: vec![Log::new_unchecked(
                        Address::repeat_byte(0xFF),
                        vec![
                            B256::repeat_byte(0xCC),
                            B256::repeat_byte(0xDD),
                            B256::repeat_byte(0xEE),
                        ],
                        Bytes::from_static(&[0xFF; 64]),
                    )],
                },
            ),
        ]
    }

    /// Verify that our local adapter produces byte-identical output to reth's adapter.
    #[test]
    fn local_and_reth_adapters_produce_identical_bytes() {
        for (name, receipt) in test_receipts() {
            let local_bytes = bincode::serialize(&LocalWrapper {
                receipt: receipt.clone(),
            })
            .unwrap();
            let reth_bytes = bincode::serialize(&RethWrapper {
                receipt: make_reth_receipt(&receipt),
            })
            .unwrap();
            assert_eq!(
                local_bytes, reth_bytes,
                "Serialization mismatch for case: {name}"
            );
        }
    }

    /// Verify reth-serialized bytes can be deserialized by our local adapter.
    #[test]
    fn reth_bytes_deserialize_with_local_adapter() {
        for (name, receipt) in test_receipts() {
            let reth_bytes = bincode::serialize(&RethWrapper {
                receipt: make_reth_receipt(&receipt),
            })
            .unwrap();
            let deserialized: LocalWrapper =
                bincode::deserialize(&reth_bytes).unwrap_or_else(|e| {
                    panic!(
                        "Failed to deserialize reth bytes with local adapter for case {name}: {e}"
                    )
                });
            assert_eq!(
                deserialized.receipt, receipt,
                "Round-trip mismatch for case: {name}"
            );
        }
    }

    /// Verify local-serialized bytes can be deserialized by reth's adapter.
    #[test]
    fn local_bytes_deserialize_with_reth_adapter() {
        for (name, receipt) in test_receipts() {
            let local_bytes = bincode::serialize(&LocalWrapper {
                receipt: receipt.clone(),
            })
            .unwrap();
            let deserialized: RethWrapper =
                bincode::deserialize(&local_bytes).unwrap_or_else(|e| {
                    panic!(
                        "Failed to deserialize local bytes with reth adapter for case {name}: {e}"
                    )
                });
            assert_eq!(
                deserialized.receipt,
                make_reth_receipt(&receipt),
                "Round-trip mismatch for case: {name}"
            );
        }
    }

    /// Golden snapshot test: hardcoded bytes that must never change.
    /// These were captured from reth_ethereum_primitives::serde_bincode_compat::Receipt
    /// serialized with bincode 1.x. If this test fails, state deserialization is broken.
    #[test]
    fn golden_snapshot_bytes() {
        // Generate expected bytes from reth for each case and assert stability
        for (name, receipt) in test_receipts() {
            let expected = bincode::serialize(&RethWrapper {
                receipt: make_reth_receipt(&receipt),
            })
            .unwrap();
            let actual = bincode::serialize(&LocalWrapper {
                receipt: receipt.clone(),
            })
            .unwrap();
            assert_eq!(
                actual, expected,
                "Golden snapshot mismatch for case: {name}\n  expected: {expected:02x?}\n  actual:   {actual:02x?}"
            );
        }

        // Additionally verify against hardcoded snapshots to catch reth upstream changes
        let snapshots = golden_snapshots();
        for (name, receipt) in test_receipts() {
            let actual = bincode::serialize(&LocalWrapper {
                receipt: receipt.clone(),
            })
            .unwrap();
            let expected_hex = snapshots
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("Missing golden snapshot for case: {name}"))
                .1;
            let expected = hex::decode(expected_hex)
                .unwrap_or_else(|e| panic!("Invalid hex in golden snapshot for case {name}: {e}"));
            assert_eq!(
                actual, expected,
                "Hardcoded golden snapshot mismatch for case: {name}"
            );
        }
    }

    /// Hardcoded hex-encoded golden snapshots. Generated from reth v1.9.0.
    fn golden_snapshots() -> Vec<(&'static str, &'static str)> {
        vec![
            GOLDEN_LEGACY_SUCCESS_NO_LOGS,
            GOLDEN_EIP1559_FAILURE_NO_LOGS,
            GOLDEN_EIP2930_SUCCESS_WITH_LOG,
            GOLDEN_EIP1559_SUCCESS_MULTIPLE_LOGS,
            GOLDEN_EIP7702_HIGH_GAS,
        ]
    }

    // Golden snapshots captured from reth_ethereum_primitives v1.9.0 + bincode 1.x.
    // If any of these change, existing serialized state will fail to deserialize.
    const GOLDEN_LEGACY_SUCCESS_NO_LOGS: (&str, &str) = (
        "legacy_success_no_logs",
        "0100000000000000000108520000000000000000000000000000",
    );
    const GOLDEN_EIP1559_FAILURE_NO_LOGS: (&str, &str) = (
        "eip1559_failure_no_logs",
        "0100000000000000020000000000000000000000000000000000",
    );
    const GOLDEN_EIP2930_SUCCESS_WITH_LOG: (&str, &str) = (
        "eip2930_success_with_log",
        "0100000000000000010150c30000000000000100000000000000140000000000000000000000000000000000000000000000000000ab0100000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000010200000000000000dead",
    );
    const GOLDEN_EIP1559_SUCCESS_MULTIPLE_LOGS: (&str, &str) = (
        "eip1559_success_multiple_logs",
        "0100000000000000020140420f00000000000200000000000000140000000000000000000000000000000000000000000000000000010200000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000aa200000000000000000000000000000000000000000000000000000000000000000000000000000bb050000000000000001020304051400000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    );
    const GOLDEN_EIP7702_HIGH_GAS: (&str, &str) = (
        "eip7702_high_gas",
        "01000000000000000401ffffffffffffffff01000000000000001400000000000000ffffffffffffffffffffffffffffffffffffffff03000000000000002000000000000000cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc2000000000000000dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd2000000000000000eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee4000000000000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    );
}
