use std::rc::Rc;

use borsh::ser::BorshSerialize;
use sov_mock_da::verifier::MockDaSpec;
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::transaction::Transaction;
pub use sov_modules_api::EncodeCall;
use sov_modules_api::{Context, DaSpec, Module, RollupAddress, Spec};
use sov_modules_stf_blueprint::{Batch, BatchReceipt, RawTx, TxEffect};

pub mod bank_data;
pub mod value_setter_data;

pub fn new_test_blob_from_batch(
    batch: Batch,
    address: &[u8],
    hash: [u8; 32],
) -> <MockDaSpec as DaSpec>::BlobTransaction {
    let address = MockAddress::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    MockBlob::new(data, address, hash)
}

pub fn has_tx_events<A: RollupAddress>(
    apply_blob_outcome: &BatchReceipt<sov_modules_stf_blueprint::SequencerOutcome<A>, TxEffect>,
) -> bool {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    events.peekable().peek().is_some()
}

/// A generic message object used to create transactions.
pub struct Message<C: Context, Mod: Module> {
    /// The sender's private key.
    pub sender_key: Rc<<C as Spec>::PrivateKey>,
    /// The message content.
    pub content: Mod::CallMessage,
    /// The ID of the chain.
    pub chain_id: u64,
    /// The gas tip for the sequencer.
    pub gas_tip: u64,
    /// The gas limit for the transaction execution.
    pub gas_limit: u64,
    /// The message nonce.
    pub nonce: u64,
}

impl<C: Context, Mod: Module> Message<C, Mod> {
    fn new(
        sender_key: Rc<<C as Spec>::PrivateKey>,
        content: Mod::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
    ) -> Self {
        Self {
            sender_key,
            content,
            chain_id,
            gas_tip,
            gas_limit,
            nonce,
        }
    }
}

/// Trait used to generate messages from the DA layer to automate module testing
pub trait MessageGenerator {
    /// Module where the messages originate from.
    type Module: Module;

    /// Module context
    type Context: Context;

    /// Generates a list of messages originating from the module.
    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>>;

    /// Creates a transaction object associated with a call message, for a given module.
    #[allow(clippy::too_many_arguments)]
    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        // Private key of the sender
        sender: &<Self::Context as Spec>::PrivateKey,
        // The message itself
        message: <Self::Module as Module>::CallMessage,
        // The ID of the chain
        chain_id: u64,
        // A gas tip for the sequencer
        gas_tip: u64,
        // The gas limit for the transaction execution
        gas_limit: u64,
        // The message nonce
        nonce: u64,
        // A boolean that indicates whether this message is the last one to be sent.
        // Useful to perform some operations specifically on the last message.
        is_last: bool,
    ) -> Transaction<Self::Context>;

    /// Creates a vector of raw transactions from the module.
    fn create_raw_txs<Encoder: EncodeCall<Self::Module>>(&self) -> Vec<RawTx> {
        let mut messages_iter = self.create_messages().into_iter().peekable();
        let mut serialized_messages = Vec::default();
        while let Some(message) = messages_iter.next() {
            let is_last = messages_iter.peek().is_none();

            let tx = self.create_tx::<Encoder>(
                &message.sender_key,
                message.content,
                message.chain_id,
                message.gas_tip,
                message.gas_limit,
                message.nonce,
                is_last,
            );

            serialized_messages.push(RawTx {
                data: tx.try_to_vec().unwrap(),
            })
        }
        serialized_messages
    }

    fn create_blobs<Encoder: EncodeCall<Self::Module>>(&self) -> Vec<u8> {
        let txs: Vec<Vec<u8>> = self
            .create_raw_txs::<Encoder>()
            .into_iter()
            .map(|tx| tx.data)
            .collect();

        txs.try_to_vec().unwrap()
    }
}
