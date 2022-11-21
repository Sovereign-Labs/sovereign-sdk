use bytes::Bytes;

use crate::{
    core::traits::{Address, Block, Blockheader},
    zk_utils::traits::Proof,
};

pub trait StateTransitionFunction {
    type Address: Address;
    type StateRoot;
    type ChainParams;
    type Transaction;
    type Block: Block<Header = Self::Header, Transaction = Self::Transaction>;
    type Proof;
    type Error;
    /// The header of a rollup block
    type Header: Blockheader;
    /// A proof that the sequencer has misbehaved. For example, this could be a merkle proof of a transaction
    /// with an invalid signature
    type MisbehaviorProof;

    /// Called at the beginning of each DA-layer block - whether or not that block contains any
    /// data relevant to the rollup
    fn begin_slot(&mut self);

    /// Parses a sequence of bytes into a rollup block if it meets some basic validity conditions
    /// (for example - if the sender is bonded on the rollup). If the sender was bonded but the block is illegal
    /// the rollup may slash the sender
    fn parse_block(
        &mut self,
        msg: Bytes,
        sender: Bytes,
    ) -> Result<Self::Block, Option<ConsensusSetUpdate<Bytes>>>;

    /// Parses a sequence of bytes into a zero-knowledge proof if the message meets some basic validity conditions
    /// (for example - if the sender is bonded on the rollup). If the sender was bonded but the message is illegal
    /// the rollup may slash the sender
    fn parse_proof(
        &mut self,
        msg: Bytes,
        sender: Bytes,
    ) -> Result<Self::Proof, Option<ConsensusSetUpdate<Bytes>>>;

    /// Called once at the beginning of each rollup block (so, potentially many times per DA block).
    /// This method has two purposes: to allow the rollup to perform and needed initialiation before
    /// processing the block, and to process an optional "misbehavior proof" to allow short-circuiting
    /// in case the block is invalid. (An example misbehavior proof would be a merkle-proof to a transaction)
    /// with an invalid signature. In case of misbehavior, this method should slash the block's sender.
    ///
    /// TODO: decide whether to add events
    fn begin_block(
        &mut self,
        block: &Self::Block,
        sender: Bytes,
        misbehavior: Option<Self::MisbehaviorProof>,
    ) -> Result<(), ConsensusSetUpdate<Bytes>>;

    /// The core of the state transition function - called once for each rollup transaction.
    ///
    /// TODO: consider simplifying the response to a `MinDeliverTxResponse` for greater efficiency in zkVM
    /// TODO: decide if events/logs need to be included in the zk-proof
    fn deliver_tx(&mut self, tx: Self::Transaction) -> AugmentedDeliverTxResponse;

    /// Called once at the end of each rollup block.
    fn end_block(&mut self) -> EndBlockResponse<Bytes>;

    /// Called once at the "end" of each DA layer block (i.e. after all rollup blocks have been processed)
    fn end_slot(&mut self) -> Self::StateRoot;

    /// Called once to update the state of the rollup with an on-chain proof. This method is useful for
    /// updating gas costs - for example, by keeping an estimate of the lag time between when a transaction
    /// is submitted and when it is proved,
    fn deliver_proof(
        &mut self,
        proof: Self::Proof,
        sender: Bytes,
    ) -> Result<DeliverProofResponse, Option<ConsensusSetUpdate<Bytes>>>;
}

/// A minimal response to a deliver_tx invocation. Contains
/// the information required to validate the chain, but does
/// not include indexing information
///
/// TODO: decide whether to add events
pub struct MinDeliverTxResponse {
    /// the response code. 0 indicates success.
    pub code: u32,
    pub data: Bytes,
    /// The amount of computational gas reserved by the transaction
    pub gas_wanted: i64,
    /// The amount of computational gas consumed by the transaction
    pub gas_used: i64,
    /// The amount of storage diesel reserved by the transaction
    pub diesel_wanted: i64,
    /// The amount of storage diesel used by the transaction
    pub diesel_used: i64,
}

/// A full response to a deliver_tx invocation, including supplemental data
/// useful for indexing.
pub struct AugmentedDeliverTxResponse {
    pub core: MinDeliverTxResponse,
    /// Key-value pairs representing changes to rollup state
    pub events: Vec<Event>,
    /// Free-form strings to allow additional output from the rollup
    pub logs: Vec<String>,
}

/// A key-value pair representing a change to the rollup state
pub struct Event {
    pub key: Bytes,
    pub value: Bytes,
}

pub struct EndBlockResponse<Addr> {
    pub sequencer_updates: Vec<ConsensusSetUpdate<Addr>>,
    pub prover_updates: Vec<ConsensusSetUpdate<Addr>>,
}

pub struct DeliverProofResponse {
    /// The amount of computational gas used
    pub gas_proved: i64,
    /// The amount of storage diesel used
    pub diesel_proved: i64,
}

pub struct ConsensusSetUpdate<Addr> {
    pub owner: Addr,
    pub power: u64,
}

pub enum ConsensusMsg<P, B> {
    Proof(P),
    Block(B),
}
