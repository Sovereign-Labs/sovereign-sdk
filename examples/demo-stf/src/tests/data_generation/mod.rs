use std::rc::Rc;

use borsh::BorshSerialize;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::PublicKey;
use sov_modules_stf_template::RawTx;

use crate::runtime::Runtime;

mod election_data;
mod value_setter_data;

pub fn simulate_da(
    value_setter_admin: DefaultPrivateKey,
    election_admin: DefaultPrivateKey,
) -> Vec<RawTx> {
    let election = election_data::ElectionCallMessages::new(election_admin);

    let mut messages = Vec::default();
    messages.extend(election.create_raw_txs());

    let value_setter = value_setter_data::ValueSetterMessages::new(value_setter_admin);
    messages.extend(value_setter.create_raw_txs());

    messages
}

pub fn simulate_da_with_revert_msg(election_admin: DefaultPrivateKey) -> Vec<RawTx> {
    let election = election_data::InvalidElectionCallMessages::new(election_admin);
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_sig(election_admin: DefaultPrivateKey) -> Vec<RawTx> {
    let election = election_data::BadSigElectionCallMessages::new(election_admin);
    election.create_raw_txs()
}

// TODO: Remove once we fix test with bad nonce
//   https://github.com/Sovereign-Labs/sovereign-sdk/issues/235
#[allow(unused)]
pub fn simulate_da_with_bad_nonce(election_admin: DefaultPrivateKey) -> Vec<RawTx> {
    let election = election_data::BadNonceElectionCallMessages::new(election_admin);
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_serialization(election_admin: DefaultPrivateKey) -> Vec<RawTx> {
    let election = election_data::BadSerializationElectionCallMessages::new(election_admin);
    election.create_raw_txs()
}

trait MessageGenerator {
    type Call;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)>;

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<DefaultContext>;

    fn create_raw_txs(&self) -> Vec<RawTx> {
        let mut messages_iter = self.create_messages().into_iter().peekable();
        let mut serialized_messages = Vec::default();
        while let Some((sender, m, nonce)) = messages_iter.next() {
            let is_last = messages_iter.peek().is_none();

            let tx = self.create_tx(&sender, m, nonce, is_last);

            serialized_messages.push(RawTx {
                data: tx.try_to_vec().unwrap(),
            })
        }
        serialized_messages
    }
}
