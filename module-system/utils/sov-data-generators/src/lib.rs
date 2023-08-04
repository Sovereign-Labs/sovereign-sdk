use std::rc::Rc;

use borsh::ser::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
pub use sov_modules_api::EncodeCall;
use sov_modules_api::{Address, Context, Module};
use sov_modules_stf_template::{Batch, RawTx, SequencerOutcome, TxEffect};
use sov_rollup_interface::digest::Digest;
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

pub trait MessageGenerator {
    type Module: Module;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Self::Module as Module>::CallMessage,
        u64,
    )>;

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Self::Module as Module>::CallMessage,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext>;

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
