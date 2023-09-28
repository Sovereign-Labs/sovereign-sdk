//! This module is the core of the Sovereign SDK. It defines the traits and types that
//! allow the SDK to run the "business logic" of any application generically.
//!
//! The most important trait in this module is the [`StateTransitionFunction`], which defines the
//! main event loop of the rollup.
use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::da::DaSpec;
use crate::zk::{ValidityCondition, Zkvm};

#[cfg(any(all(test, feature = "sha2"), feature = "fuzzing"))]
pub mod fuzzing;

/// The configuration of a full node of the rollup which creates zk proofs.
pub struct ProverConfig;
/// The configuration used to initialize the "Verifier" of the state transition function
/// which runs inside of the zkvm.
pub struct ZkConfig;
/// The configuration of a standard full node of the rollup which does not create zk proofs
pub struct StandardConfig;

/// A special marker trait which allows us to define different rollup configurations. There are
/// only 3 possible instantiations of this trait: [`ProverConfig`], [`ZkConfig`], and [`StandardConfig`].
pub trait StateTransitionConfig: sealed::Sealed {}
impl StateTransitionConfig for ProverConfig {}
impl StateTransitionConfig for ZkConfig {}
impl StateTransitionConfig for StandardConfig {}

// https://rust-lang.github.io/api-guidelines/future-proofing.html
mod sealed {
    use super::{ProverConfig, StandardConfig, ZkConfig};

    pub trait Sealed {}
    impl Sealed for ProverConfig {}
    impl Sealed for ZkConfig {}
    impl Sealed for StandardConfig {}
}

/// A receipt for a single transaction. These receipts are stored in the rollup's database
/// and may be queried via RPC. Receipts are generic over a type `R` which the rollup can use to
/// store additional data, such as the status code of the transaction or the amout of gas used.s
#[derive(Debug, Clone, Serialize, Deserialize)]
/// A receipt showing the result of a transaction
pub struct TransactionReceipt<R> {
    /// The canonical hash of this transaction
    pub tx_hash: [u8; 32],
    /// The canonically serialized body of the transaction, if it should be persisted
    /// in the database
    pub body_to_save: Option<Vec<u8>>,
    /// The events output by this transaction
    pub events: Vec<Event>,
    /// Any additional structured data to be saved in the database and served over RPC
    /// For example, this might contain a status code.
    pub receipt: R,
}

/// A receipt for a batch of transactions. These receipts are stored in the rollup's database
/// and may be queried via RPC. Batch receipts are generic over a type `BatchReceiptContents` which the rollup
/// can use to store arbitrary typed data, like the gas used by the batch. They are also generic over a type `TxReceiptContents`,
/// since they contain a vectors of [`TransactionReceipt`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// A receipt giving the outcome of a batch of transactions
pub struct BatchReceipt<BatchReceiptContents, TxReceiptContents> {
    /// The canonical hash of this batch
    pub batch_hash: [u8; 32],
    /// The receipts of all the transactions in this batch.
    pub tx_receipts: Vec<TransactionReceipt<TxReceiptContents>>,
    /// Any additional structured data to be saved in the database and served over RPC
    pub inner: BatchReceiptContents,
}

/// Result of applying a slot to current state
/// Where:
///  - S - generic for state root
///  - B - generic for batch receipt contents
///  - T - generic for transaction receipt contents
///  - W - generic for witness
pub struct SlotResult<S, B, T, W> {
    /// Final state root after all blobs were applied
    pub state_root: S,
    /// Receipt for each applied batch
    pub batch_receipts: Vec<BatchReceipt<B, T>>,
    /// Witness after applying the whole block
    pub witness: W,
}

// TODO(@preston-evans98): update spec with simplified API
/// State transition function defines business logic that responsible for changing state.
/// Terminology:
///  - state root: root hash of state merkle tree
///  - block: DA layer block
///  - batch: Set of transactions grouped together, or block on L2
///  - blob: Non serialised batch or anything else that can be posted on DA layer, like attestation or proof.
pub trait StateTransitionFunction<Vm: Zkvm, Da: DaSpec> {
    /// Root hash of state merkle tree
    type StateRoot: Serialize + DeserializeOwned + Clone;
    /// The initial state of the rollup.
    type InitialState;

    /// The contents of a transaction receipt. This is the data that is persisted in the database
    type TxReceiptContents: Serialize + DeserializeOwned + Clone;

    /// The contents of a batch receipt. This is the data that is persisted in the database
    type BatchReceiptContents: Serialize + DeserializeOwned + Clone;

    /// Witness is a data that is produced during actual batch execution
    /// or validated together with proof during verification
    type Witness: Default + Serialize + DeserializeOwned;

    /// The validity condition that must be verified outside of the Vm
    type Condition: ValidityCondition;

    /// Perform one-time initialization for the genesis block and returns the resulting root hash wrapped in a result.
    /// If the init chain fails we panic.
    fn init_chain(&mut self, params: Self::InitialState) -> Self::StateRoot;

    /// Called at each **DA-layer block** - whether or not that block contains any
    /// data relevant to the rollup.
    /// If slot is started in Full Node mode, default witness should be provided.
    /// If slot is started in Zero Knowledge mode, witness from execution should be provided.
    ///
    /// Applies batches of transactions to the rollup,
    /// slashing the sequencer who proposed the blob on failure.
    /// The blobs are contained into a slot whose data is contained within the `slot_data` parameter,
    /// this parameter is mainly used within the begin_slot hook.
    /// The concrete blob type is defined by the DA layer implementation,
    /// which is why we use a generic here instead of an associated type.
    ///
    /// Commits state changes to the database
    fn apply_slot<'a, I>(
        &mut self,
        pre_state_root: &Self::StateRoot,
        witness: Self::Witness,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>;
}

/// A key-value pair representing a change to the rollup state
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(proptest_derive::Arbitrary))]
pub struct Event {
    key: EventKey,
    value: EventValue,
}

impl Event {
    /// Create a new event with the given key and value
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: EventKey(key.as_bytes().to_vec()),
            value: EventValue(value.as_bytes().to_vec()),
        }
    }

    /// Get the event key
    pub fn key(&self) -> &EventKey {
        &self.key
    }

    /// Get the event value
    pub fn value(&self) -> &EventValue {
        &self.value
    }
}

/// The key of an event. This is a wrapper around a `Vec<u8>`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(proptest_derive::Arbitrary))]
pub struct EventKey(Vec<u8>);

impl EventKey {
    /// Return the inner bytes of the event key.
    pub fn inner(&self) -> &Vec<u8> {
        &self.0
    }
}

/// The value of an event. This is a wrapper around a `Vec<u8>`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(proptest_derive::Arbitrary))]
pub struct EventValue(Vec<u8>);

impl EventValue {
    /// Return the inner bytes of the event value.
    pub fn inner(&self) -> &Vec<u8> {
        &self.0
    }
}
