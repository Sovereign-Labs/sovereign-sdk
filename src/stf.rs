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
    type Header: Blockheader;
    /// A proof that the sequencer has misbehaved. For example, this could be a merkle proof of a transaction
    /// with an invalid signature
    type MisbehaviorProof;

    fn begin_slot(&mut self);
    fn parse_block(
        &mut self,
        msg: Bytes,
        sender: Bytes,
    ) -> Result<Self::Block, ConsensusSetUpdate<Bytes>>;
    fn parse_proof(
        &mut self,
        msg: Bytes,
        sender: Bytes,
    ) -> Result<Self::Proof, ConsensusSetUpdate<Bytes>>;
    /// TODO: decide whether to add events
    fn begin_block(
        &mut self,
        header: &Self::Header,
        misbehavior: Option<Self::MisbehaviorProof>,
    ) -> Result<(), ConsensusSetUpdate<Bytes>>;
    fn deliver_tx(&mut self, tx: Self::Transaction) -> MinDeliverTxResponse;

    fn end_block(&mut self) -> EndBlockResponse<Bytes>;
    fn end_slot(&mut self) -> Self::StateRoot;
    fn deliver_proof(&mut self, proof: Self::Proof, sender: Bytes) -> DeliverProofResponse;
}

/// The minimal possible response to a deliver_tx invocation.
///
/// TODO: decide whether to add events
pub struct MinDeliverTxResponse {
    /// the response code. 0 indicates success.
    pub code: u32,
    pub data: Bytes,
    pub gas_wanted: i64,
    pub gas_used: i64,
    pub diesel_wanted: i64,
    pub diesel_used: i64,
}

pub struct EndBlockResponse<Addr> {
    pub sequencer_updates: Vec<ConsensusSetUpdate<Addr>>,
    pub prover_updates: Vec<ConsensusSetUpdate<Addr>>,
}

pub struct DeliverProofResponse {
    pub gas_proved: i64,
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
