//! The rpc module defines types and traits for querying chain history
//! via an RPC interface.
#[cfg(feature = "native")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::maybestd::vec::Vec;
#[cfg(feature = "native")]
use crate::stf::Event;
use crate::stf::EventKey;

/// A struct containing enough information to uniquely specify single batch.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotIdAndOffset {
    /// The [`SlotIdentifier`] of the slot containing this batch.
    pub slot_id: SlotIdentifier,
    /// The offset into the slot at which this tx is located.
    /// Index 0 is the first batch in the slot.
    pub offset: u64,
}

/// A struct containing enough information to uniquely specify single transaction.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchIdAndOffset {
    /// The [`BatchIdentifier`] of the batch containing this transaction.
    pub batch_id: BatchIdentifier,
    /// The offset into the batch at which this tx is located.
    /// Index 0 is the first transaction in the batch.
    pub offset: u64,
}

/// A struct containing enough information to uniquely specify single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TxIdAndOffset {
    /// The [`TxIdentifier`] of the transaction containing this event.
    pub tx_id: TxIdentifier,
    /// The offset into the tx's events at which this event is located.
    /// Index 0 is the first event from this tx.
    pub offset: u64,
}

/// A struct containing enough information to uniquely specify single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TxIdAndKey {
    /// The [`TxIdentifier`] of the transaction containing this event.
    pub tx_id: TxIdentifier,
    /// The key of the event.
    pub key: EventKey,
}

/// An identifier that specifies a single batch
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BatchIdentifier {
    /// The hex-encoded hash of the batch, as computed by the DA layer.
    Hash(#[serde(with = "utils::rpc_hex")] [u8; 32]),
    /// An offset into a particular slot (i.e. the 3rd batch in slot 5).
    SlotIdAndOffset(SlotIdAndOffset),
    /// The monotonically increasing number of the batch, ordered by the DA layer For example, if the genesis slot
    /// contains 0 batches, slot 1 contains 2 txs, and slot 3 contains 3 txs,
    /// the last batch in block 3 would have number 5. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single transaction.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TxIdentifier {
    /// The hex encoded hash of the transaction.
    Hash(#[serde(with = "utils::rpc_hex")] [u8; 32]),
    /// An offset into a particular batch (i.e. the 3rd transaction in batch 5).
    BatchIdAndOffset(BatchIdAndOffset),
    /// The monotonically increasing number of the tx, ordered by the DA layer For example, if genesis
    /// contains 0 txs, batch 1 contains 8 txs, and batch 3 contains 7 txs,
    /// the last tx in batch 3 would have number 15. The counter never resets.
    Number(u64),
}

/// An identifier that specifies a single event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventIdentifier {
    /// An offset into a particular transaction (i.e. the 3rd event in transaction number 5).
    TxIdAndOffset(TxIdAndOffset),
    /// A particular event key from a particular transaction.
    TxIdAndKey(TxIdAndKey),
    /// The monotonically increasing number of the event, ordered by the DA layer For example, if the first tx
    /// contains 7 events, tx 2 contains 11 events, and tx 3 contains 7 txs,
    /// the last event in tx 3 would have number 25. The counter never resets.
    Number(u64),
}

/// An identifier for a group of related events
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventGroupIdentifier {
    /// Fetch all events from a particular transaction.
    TxId(TxIdentifier),
    /// Fetch all events (i.e. from all transactions) with a particular key.
    Key(Vec<u8>),
}

/// An identifier that specifies a single slot.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SlotIdentifier {
    /// The hex encoded hash of the slot (i.e. the da layer's block hash).
    Hash(#[serde(with = "utils::rpc_hex")] [u8; 32]),
    /// The monotonically increasing number of the slot, ordered by the DA layer but starting from 0
    /// at the *rollup's* genesis.
    Number(u64),
}

/// A QueryMode specifies how much information to return in response to an RPC query
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    /// Returns the parent struct but no details about its children.
    /// For example, a `Compact` "get_slots" response would simply state the range of batch
    /// numbers which occurred in the slot, but not the hashes of the batches themselves.
    Compact,
    /// Returns the parent struct and the hashes of all its children.
    Standard,
    /// Returns the parent struct and all its children, recursively fetching its children
    /// in `Full` mode. For example, a `Full` "get_batch" response would include the `Full`
    /// details of all the transactions in the batch, and those would in turn return the event bodies
    /// which had occurred in those transactions.
    Full,
}

impl Default for QueryMode {
    fn default() -> Self {
        Self::Standard
    }
}

/// The body of a response to a JSON-RPC request for a particular slot.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SlotResponse<B, Tx> {
    /// The slot number.
    pub number: u64,
    /// The hex encoded slot hash.
    #[serde(with = "utils::rpc_hex")]
    pub hash: [u8; 32],
    /// The range of batches in this slot.
    pub batch_range: core::ops::Range<u64>,
    /// The batches in this slot, if the [`QueryMode`] of the request is not `Compact`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batches: Option<Vec<ItemOrHash<BatchResponse<B, Tx>>>>,
}

/// The response to a JSON-RPC request for a particular batch.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BatchResponse<B, Tx> {
    /// The hex encoded batch hash.
    #[serde(with = "utils::rpc_hex")]
    pub hash: [u8; 32],
    /// The range of transactions in this batch.
    pub tx_range: core::ops::Range<u64>,
    /// The transactions in this batch, if the [`QueryMode`] of the request is not `Compact`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txs: Option<Vec<ItemOrHash<TxResponse<Tx>>>>,
    /// The custom receipt specified by the rollup. This typically contains
    /// information about the outcome of the batch.
    pub custom_receipt: B,
}

/// The response to a JSON-RPC request for a particular transaction.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct TxResponse<Tx> {
    /// The hex encoded transaction hash.
    #[serde(with = "utils::rpc_hex")]
    pub hash: [u8; 32],
    /// The range of events occurring in this transaction.
    pub event_range: core::ops::Range<u64>,
    /// The transaction body, if stored by the rollup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<u8>>,
    /// The custom receipt specified by the rollup. This typically contains
    /// information about the outcome of the transaction.
    pub custom_receipt: Tx,
}

/// An RPC response which might contain a full item or just its hash.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemOrHash<T> {
    /// The hex encoded hash of the requested item.
    Hash(#[serde(with = "utils::rpc_hex")] [u8; 32]),
    /// The full item body.
    Full(T),
}

/// A LedgerRpcProvider provides a way to query the ledger for information about slots, batches, transactions, and events.
#[cfg(feature = "native")]
pub trait LedgerRpcProvider {
    /// Get the latest slot in the ledger.
    fn get_head<B: DeserializeOwned + Clone, T: DeserializeOwned>(
        &self,
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;

    /// Get a list of slots by id. The IDs need not be ordered.
    fn get_slots<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        slot_ids: &[SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;

    /// Get a list of batches by id. The IDs need not be ordered.
    fn get_batches<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        batch_ids: &[BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;

    /// Get a list of transactions by id. The IDs need not be ordered.
    fn get_transactions<T: DeserializeOwned>(
        &self,
        tx_ids: &[TxIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;

    /// Get events by id. The IDs need not be ordered.
    fn get_events(
        &self,
        event_ids: &[EventIdentifier],
    ) -> Result<Vec<Option<Event>>, anyhow::Error>;

    /// Get a single slot by hash.
    fn get_slot_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;

    /// Get a single batch by hash.
    fn get_batch_by_hash<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;

    /// Get a single transaction by hash.
    fn get_tx_by_hash<T: DeserializeOwned>(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;

    /// Get a single slot by number.
    fn get_slot_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<SlotResponse<B, T>>, anyhow::Error>;

    /// Get a single batch by number.
    fn get_batch_by_number<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<BatchResponse<B, T>>, anyhow::Error>;

    /// Get a single event by number.
    fn get_event_by_number(&self, number: u64) -> Result<Option<Event>, anyhow::Error>;

    /// Get a single tx by number.
    fn get_tx_by_number<T: DeserializeOwned>(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<TxResponse<T>>, anyhow::Error>;

    /// Get a range of slots. This query is the most efficient way to
    /// fetch large numbers of slots, since it allows for easy batching of
    /// db queries for adjacent items.
    fn get_slots_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<SlotResponse<B, T>>>, anyhow::Error>;

    /// Get a range of batches. This query is the most efficient way to
    /// fetch large numbers of batches, since it allows for easy batching of
    /// db queries for adjacent items.
    fn get_batches_range<B: DeserializeOwned, T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<BatchResponse<B, T>>>, anyhow::Error>;

    /// Get a range of batches. This query is the most efficient way to
    /// fetch large numbers of transactions, since it allows for easy batching of
    /// db queries for adjacent items.
    fn get_transactions_range<T: DeserializeOwned>(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<TxResponse<T>>>, anyhow::Error>;

    /// Get a notification each time a slot is processed
    fn subscribe_slots(&self) -> Result<tokio::sync::broadcast::Receiver<u64>, anyhow::Error>;
}

/// JSON-RPC -related utilities. Occasionally useful but unimportant for most
/// use cases.
pub mod utils {
    /// Serialization and deserialization logic for `0x`-prefixed hex strings.
    pub mod rpc_hex {
        use core::fmt;
        use core::marker::PhantomData;

        use hex::{FromHex, ToHex};
        use serde::de::{Error, Visitor};
        use serde::{Deserializer, Serializer};

        use crate::maybestd::format;
        use crate::maybestd::string::String;

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
}

#[cfg(test)]
mod rpc_hex_tests {
    use serde::{Deserialize, Serialize};

    use crate::maybestd::vec;
    use crate::maybestd::vec::Vec;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestStruct {
        #[serde(with = "super::utils::rpc_hex")]
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
