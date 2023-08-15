#[cfg(feature = "native")]
use std::rc::Rc;

use borsh::ser::BorshSerialize;
#[cfg(feature = "native")]
use sov_modules_api::transaction::Transaction;
use sov_modules_api::Address;
pub use sov_modules_api::EncodeCall;
#[cfg(feature = "native")]
use sov_modules_api::{Context, Module, Spec};
#[cfg(feature = "native")]
use sov_modules_stf_template::RawTx;
use sov_modules_stf_template::{Batch, SequencerOutcome, TxEffect};
use sov_rollup_interface::mocks::TestBlob;
use sov_rollup_interface::stf::BatchReceipt;
use sov_rollup_interface::AddressTrait;

#[cfg(feature = "native")]
pub mod bank_data;
#[cfg(feature = "native")]
pub mod election_data;
#[cfg(feature = "native")]
pub mod value_setter_data;

pub fn new_test_blob_from_batch(batch: Batch, address: &[u8], hash: [u8; 32]) -> TestBlob<Address> {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address, hash)
}

pub fn has_tx_events<A: AddressTrait>(
    apply_blob_outcome: &BatchReceipt<SequencerOutcome<A>, TxEffect>,
) -> bool {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    events.peekable().peek().is_some()
}

#[cfg(feature = "native")]
/// A generic message object used to create transactions.
pub struct Message<C: Context, Mod: Module> {
    /// The sender's private key.
    pub sender_key: Rc<<C as Spec>::PrivateKey>,
    /// The message content.
    pub content: Mod::CallMessage,
    /// The message nonce.
    pub nonce: u64,
}

#[cfg(feature = "native")]
impl<C: Context, Mod: Module> Message<C, Mod> {
    fn new(sender_key: Rc<<C as Spec>::PrivateKey>, content: Mod::CallMessage, nonce: u64) -> Self {
        Self {
            sender_key,
            content,
            nonce,
        }
    }
}

#[cfg(feature = "native")]
/// Trait used to generate messages from the DA layer to automate module testing
pub trait MessageGenerator {
    /// Module where the messages originate from.
    type Module: Module;

    /// Module context
    type Context: Context;

    /// Generates a list of messages originating from the module.
    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>>;

    /// Creates a transaction object associated with a call message, for a given module.
    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        // Private key of the sender
        sender: &<Self::Context as Spec>::PrivateKey,
        // The message itself
        message: <Self::Module as Module>::CallMessage,
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
                message.nonce,
                is_last,
            );

            serialized_messages.push(RawTx {
                data: tx.try_to_vec().unwrap(),
            })
        }
        serialized_messages
    }
}
