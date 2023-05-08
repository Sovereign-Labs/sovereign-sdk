use crate::{da::BlobTransactionTrait, maybestd::rc::Rc};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::serial::DecodeBorrowed;

/// An address on the DA layer. Opaque to the StateTransitionFunction
pub type OpaqueAddress = Rc<Vec<u8>>;

/// The configuration of a full node of the rollup which creates zk proofs.
pub struct ProverConfig;
/// The configuration used to initialize the "Verifier" of the state transition function
/// which runs inside of the zkvm.
pub struct ZkConfig;
/// The configuration of a standard full node of the rollup which does not create zk proofs
pub struct StandardConfig;

pub trait StateTransitionConfig: sealed::Sealed {}
impl StateTransitionConfig for ProverConfig {}
impl StateTransitionConfig for ZkConfig {}
impl StateTransitionConfig for StandardConfig {}

mod sealed {
    use super::{ProverConfig, StandardConfig, ZkConfig};

    pub trait Sealed {}
    impl Sealed for ProverConfig {}
    impl Sealed for ZkConfig {}
    impl Sealed for StandardConfig {}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReceipt<BatchReceiptContents, TxReceiptContents> {
    /// The canonical hash of this batch
    pub batch_hash: [u8; 32],
    /// The receipt of each transaction in the batch
    pub tx_receipts: Vec<TransactionReceipt<TxReceiptContents>>,
    /// Any additional structered data to be saved in the database and served over RPC
    pub inner: BatchReceiptContents,
}

// TODO(@preston-evans98): update spec with simplified API
/// State transition function defines business logic that responsible for changing state.
/// Terminology:
///  - state root: root hash of state merkle tree
///  - block: DA layer block
///  - batch: Set of transactions grouped together, or block on L2
///  - blob: Non serialised batch
pub trait StateTransitionFunction {
    type StateRoot;
    /// The initial state of the rollup.
    type InitialState;

    // TODO: remove unused types and their corresponding traits
    // type Transaction: TransactionTrait;
    // /// A batch of transactions. Also known as a "block" in most systems: we use
    // /// the term batch in this context to avoid ambiguity with DA layer blocks
    // type Batch: BatchTrait<Transaction = Self::Transaction>;
    // type Proof: Decode;

    /// The contents of a transaction receipt. This is the data that is persisted in the database
    type TxReceiptContents: Serialize + DeserializeOwned + Clone;
    /// The contents of a batch receipt. This is the data that is persisted in the database
    type BatchReceiptContents: Serialize + DeserializeOwned + Clone;

    /// Witness is a data that is produced during actual batch execution
    /// or validated together with proof during verification
    type Witness: Default;

    /// A proof that the sequencer has misbehaved. For example, this could be a merkle proof of a transaction
    /// with an invalid signature
    type MisbehaviorProof;

    /// Perform one-time initialization for the genesis block.
    fn init_chain(&mut self, params: Self::InitialState);

    /// Called at the beginning of each DA-layer block - whether or not that block contains any
    /// data relevant to the rollup.
    /// If slot is started in Node context, default witness should be provided
    /// if slot is tarted in Zero Knowledge context, witness from execution should be provided
    fn begin_slot(&mut self, witness: Self::Witness);

    /// Apply a blob/batch of transactions to the rollup, slashing the sequencer who proposed the blob on failure.
    /// The concrete blob type is defined by the DA layer implementation, which is why we use a generic here instead
    /// of an associated type.
    fn apply_blob(
        &mut self,
        blob: impl BlobTransactionTrait,
        misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> BatchReceipt<Self::BatchReceiptContents, Self::TxReceiptContents>;

    /// Called once at the *end* of each DA layer block (i.e. after all rollup blob have been processed)
    /// Commits state changes to the database
    ///
    fn end_slot(
        &mut self,
    ) -> (
        Self::StateRoot,
        Self::Witness,
        Vec<ConsensusSetUpdate<OpaqueAddress>>,
    );
}

pub trait StateTransitionRunner<T: StateTransitionConfig> {
    /// The parameters of the state transition function which are set at runtime. For example,
    /// the runtime config might contain path to a data directory.
    type RuntimeConfig;
    type Inner: StateTransitionFunction;
    // TODO: decide if `new` also requires <Self as StateTransitionFunction>::ChainParams as an argument
    /// Create a state transition runner
    fn new(runtime_config: Self::RuntimeConfig) -> Self;

    /// Return a reference to the inner STF implementation
    fn inner(&self) -> &Self::Inner;

    /// Return a mutable reference to the inner STF implementation
    fn inner_mut(&mut self) -> &mut Self::Inner;

    // /// Report if the state transition function has been initialized.
    // /// If not, node implementations should respond by running `init_chain`
    // fn has_been_initialized(&self) -> bool;
}

#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize)]
pub enum ConsensusRole {
    Prover,
    Sequencer,
    ProverAndSequencer,
}

/// A key-value pair representing a change to the rollup state
#[derive(Debug, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone)]
pub struct Event {
    pub key: EventKey,
    pub value: EventValue,
}

impl Event {
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: EventKey(key.as_bytes().to_vec()),
            value: EventValue(value.as_bytes().to_vec()),
        }
    }
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Clone,
    Serialize,
    Deserialize,
)]
pub struct EventKey(Vec<u8>);

#[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone)]
pub struct EventValue(Vec<u8>);

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ConsensusSetUpdate<Address> {
    pub address: Address,
    pub new_role: Option<ConsensusRole>,
}

impl ConsensusSetUpdate<OpaqueAddress> {
    pub fn slashing(sequencer: &[u8]) -> ConsensusSetUpdate<OpaqueAddress> {
        let faulty_sequencer = Rc::new(sequencer.to_vec());
        ConsensusSetUpdate {
            address: faulty_sequencer,
            new_role: None,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ConsensusMessage<B, P> {
    Batch(B),
    Proof(P),
}

#[derive(Debug, PartialEq, Clone)]
pub enum ConsensusMessageDecodeError<BatchErr, ProofErr> {
    Batch(BatchErr),
    Proof(ProofErr),
    NoTag,
    InvalidTag { max_allowed: u8, got: u8 },
}

impl<'de, P: DecodeBorrowed<'de>, B: DecodeBorrowed<'de>> DecodeBorrowed<'de>
    for ConsensusMessage<B, P>
{
    type Error = ConsensusMessageDecodeError<B::Error, P::Error>;
    fn decode_from_slice(target: &'de [u8]) -> Result<Self, Self::Error> {
        Ok(
            match *target
                .iter()
                .next()
                .ok_or(ConsensusMessageDecodeError::NoTag)?
            {
                0 => Self::Batch(
                    B::decode_from_slice(&target[1..])
                        .map_err(ConsensusMessageDecodeError::Batch)?,
                ),
                1 => Self::Proof(
                    P::decode_from_slice(&target[1..])
                        .map_err(ConsensusMessageDecodeError::Proof)?,
                ),
                _ => Err(ConsensusMessageDecodeError::InvalidTag {
                    max_allowed: 1,
                    got: target[0],
                })?,
            },
        )
    }
}
