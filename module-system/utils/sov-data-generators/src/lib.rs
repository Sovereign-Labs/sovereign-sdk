use std::rc::Rc;

use borsh::ser::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
pub use sov_modules_api::EncodeCall;
use sov_modules_api::{Address, Context, Module};
use sov_modules_stf_template::{Batch, RawTx, SequencerOutcome, TxEffect};
use sov_rollup_interface::mocks::TestBlob;
use sov_rollup_interface::stf::BatchReceipt;
pub mod bank_data;
pub mod election_data;
pub mod value_setter_data;

pub fn new_test_blob_from_batch(batch: Batch, address: &[u8], hash: [u8; 32]) -> TestBlob<Address> {
    let address = Address::try_from(address).unwrap();
    let data = batch.try_to_vec().unwrap();
    TestBlob::new(data, address, hash)
}

pub fn has_tx_events(apply_blob_outcome: &BatchReceipt<SequencerOutcome, TxEffect>) -> bool {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    events.peekable().peek().is_some()
}

/// Trait used to generate messages from the DA layer to automate module testing
pub trait MessageGenerator {
    /// Module where the messages originate from.
    type Module: Module;

    /// Generates a list of messages originating from the module.
    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Self::Module as Module>::CallMessage,
        u64,
    )>;

    /// Creates a transaction object associated with a call message, for a given module.
    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        // Private key of the sender
        sender: &DefaultPrivateKey,
        // The message itself
        message: <Self::Module as Module>::CallMessage,
        // The message nonce
        nonce: u64,
        // A boolean that indicates whether this message is the last one to be sent.
        // Useful to perform some operations specifically on the last message.
        is_last: bool,
    ) -> Transaction<DefaultContext>;

    /// Creates a vector of raw transactions from the module.
    fn create_raw_txs<Encoder: EncodeCall<Self::Module>>(&self) -> Vec<RawTx> {
        let mut messages_iter = self.create_messages().into_iter().peekable();
        let mut serialized_messages = Vec::default();
        while let Some((sender, m, nonce)) = messages_iter.next() {
            let is_last = messages_iter.peek().is_none();

            let tx = self.create_tx::<Encoder>(&sender, m, nonce, is_last);

            serialized_messages.push(RawTx {
                data: tx.try_to_vec().unwrap(),
            })
        }
        serialized_messages
    }
}
