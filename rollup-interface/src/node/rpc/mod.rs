use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::stf::Event;

/// An identifier that specifies a single batch
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum BatchIdentifier {
    Hash([u8; 32]),
    SlotIdAndIndex((SlotIdentifier, u64)),
    /// The monotonically increasing number of the batch, ordered by the DA layer For example, if the genesis slot
    /// contains 0 batches, slot 1 contains 2 txs, and slot 3 contains 3 txs,
    /// the last batch in block 3 would have number 5. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single transaction
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum TxIdentifier {
    Hash([u8; 32]),
    BatchIdAndIndex((BatchIdentifier, u64)),
    /// The monotonically increasing number of the tx, ordered by the DA layer For example, if genesis
    /// contains 0 txs, batch 1 contains 8 txs, and batch 3 contains 7 txs,
    /// the last tx in batch 3 would have number 15. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single event
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum EventIdentifier {
    TxIdAndIndex((TxIdentifier, u64)),
    TxIdAndKey((TxIdentifier, Vec<u8>)),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 txs,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(u64),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum EventGroupIdentifier {
    TxId(TxIdentifier),
    Key(Vec<u8>),
}

/// An identifier that specifies a single slot
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SlotIdentifier {
    Hash([u8; 32]), // the hash of a da block
    Number(u64),    // the block number of a da block
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    /// Returns the minimal parent struct with no minimal about its children.
    /// For example, a compact "slot" response would contain a range of
    Compact,
    Standard,
    Full,
}

impl Default for QueryMode {
    fn default() -> Self {
        Self::Compact
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SlotResponse<B, Tx> {
    pub number: u64,
    #[serde(with = "rpc_hex")]
    pub hash: [u8; 32],
    pub batch_range: std::ops::Range<u64>,
    pub batches: Option<Vec<ItemOrHash<BatchResponse<B, Tx>>>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BatchResponse<B, Tx> {
    #[serde(with = "rpc_hex")]
    pub hash: [u8; 32],
    pub tx_range: std::ops::Range<u64>,
    pub txs: Option<Vec<ItemOrHash<TxResponse<Tx>>>>,
    pub custom_receipt: B,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct TxResponse<Tx> {
    #[serde(with = "rpc_hex")]
    pub hash: [u8; 32],
    pub event_range: std::ops::Range<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<u8>>,
    #[serde(flatten)]
    pub custom_receipt: Tx,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemOrHash<T> {
    Hash([u8; 32]),
    Full(T),
}

pub trait LedgerRpcProvider {
    fn get_head<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;

    fn get_slots<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;
    fn get_batches<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;
    fn get_transactions<T: DeserializeOwned>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;
    fn get_events(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<Event>>, anyhow::Error>;
    fn get_slot_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;
    fn get_batch_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;
    fn get_tx_by_hash<T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;
    fn get_slot_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;
    fn get_batch_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;
    fn get_event_by_number(&self, number: u64) -> Result<Option<Event>, anyhow::Error>;
    fn get_tx_by_number<T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;
    fn get_slots_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;
    fn get_batches_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;
    fn get_transactions_range<T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;
}

mod rpc_hex {
    use core::fmt;
    use std::marker::PhantomData;

    use hex::{FromHex, ToHex};
    use serde::{
        de::{Error, Visitor},
        Deserializer, Serializer,
    };

    /// Serializes `data` as hex string using lowercase characters and prefixing with '0x'.
    ///
    /// Lowercase characters are used (e.g. `f9b4ca`). The resulting string's length
    /// is always even, each byte in data is always encoded using two hex digits.
    /// Thus, the resulting string contains exactly twice as many bytes as the input
    /// data.
    pub fn serialize<S, T>(data: T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: ToHex,
    {
        let formatted_string = format!("0x{}", data.encode_hex::<String>());
        serializer.serialize_str(&formatted_string)
    }

    /// Deserializes a hex string into raw bytes.
    ///
    /// Both, upper and lower case characters are valid in the input string and can
    /// even be mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all valid strings).
    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromHex,
        <T as FromHex>::Error: fmt::Display,
    {
        struct HexStrVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for HexStrVisitor<T>
        where
            T: FromHex,
            <T as FromHex>::Error: fmt::Display,
        {
            type Value = T;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a hex encoded string")
            }

            fn visit_str<E>(self, data: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                FromHex::from_hex(data).map_err(Error::custom)
            }

            fn visit_borrowed_str<E>(self, data: &'de str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                FromHex::from_hex(data).map_err(Error::custom)
            }
        }

        deserializer.deserialize_str(HexStrVisitor(PhantomData))
    }
}

#[cfg(test)]
mod rpc_hex_tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestStruct {
        #[serde(with = "super::rpc_hex")]
        data: Vec<u8>,
    }

    #[test]
    fn test_roundtrip() {
        let test_data = TestStruct {
            data: vec![0x01, 0x02, 0x03, 0x04],
        };

        let serialized = serde_json::to_string(&test_data).unwrap();
        assert!(serialized.contains("0x01020304"));
        let deserialized: TestStruct = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, test_data)
    }

    #[test]
    fn test_accepts_hex_without_0x_prefix() {
        let test_data = TestStruct {
            data: vec![0x01, 0x02, 0x03, 0x04],
        };

        let deserialized: TestStruct = serde_json::from_str(r#"{"data": "01020304"}"#).unwrap();
        assert_eq!(deserialized, test_data)
    }
}
