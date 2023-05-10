use std::rc::Rc;

use crate::runtime::Runtime;
use crate::tx_verifier_impl::Transaction;

use borsh::BorshSerialize;
use sov_app_template::RawTx;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::default_signature::{DefaultPublicKey, DefaultSignature};
use sov_modules_api::PublicKey;

mod election_data;
mod value_setter_data;

pub fn simulate_da(
    value_setter_admin: DefaultPrivateKey,
    election_admin: DefaultPrivateKey,
) -> Vec<RawTx> {
    let voters = vec![
        Rc::new(DefaultPrivateKey::generate()),
        Rc::new(DefaultPrivateKey::generate()),
        Rc::new(DefaultPrivateKey::generate()),
    ];

    let election = election_data::ElectionCallMessages {
        election_admin: Rc::new(election_admin),
        voters,
    };

    let mut messages = Vec::default();
    messages.extend(election.create_raw_txs());

    let value_setter = value_setter_data::ValueSetterMessages {
        admin: Rc::new(value_setter_admin),
    };
    messages.extend(value_setter.create_raw_txs());

    messages
}
/*
pub fn simulate_da_with_revert_msg() -> Vec<RawTx> {
    let election = election_data::InvalidElectionCallMessages {};
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_sig() -> Vec<RawTx> {
    let election = election_data::BadSigElectionCallMessages {};
    election.create_raw_txs()
}

// TODO: Remove once we fix test with bad nonce
//   https://github.com/Sovereign-Labs/sovereign/issues/235
#[allow(unused)]
pub fn simulate_da_with_bad_nonce() -> Vec<RawTx> {
    let election = election_data::BadNonceElectionCallMessages {};
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_serialization() -> Vec<RawTx> {
    let election = election_data::BadSerializationElectionCallMessages {};
    election.create_raw_txs()
}
*/
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

pub(crate) struct QueryGenerator {}

impl QueryGenerator {
    pub(crate) fn generate_query_election_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GetResult;
        Runtime::<DefaultContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_election_nb_of_votes_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GenNbOfVotes;
        Runtime::<DefaultContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_value_setter_message() -> Vec<u8> {
        let query_message = value_setter::query::QueryMessage::GetValue;
        Runtime::<DefaultContext>::encode_value_setter_query(query_message)
    }

    pub(crate) fn generate_query_check_balance() -> Vec<u8> {
        let query_message = sequencer::query::QueryMessage::GetSequencerAddressAndBalance;
        Runtime::<DefaultContext>::encode_sequencer_query(query_message)
    }
}
