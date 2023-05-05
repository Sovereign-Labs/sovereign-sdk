use crate::runtime::Runtime;
use crate::tx_verifier_impl::Transaction;

use borsh::BorshSerialize;
use sov_app_template::RawTx;
use sov_modules_api::mocks::{MockContext, MockPublicKey, MockSignature};
use sov_modules_api::PublicKey;

mod election_data;
mod value_setter_data;

pub fn simulate_da() -> Vec<RawTx> {
    let election = election_data::ElectionCallMessages {};

    let mut messages = Vec::default();
    messages.extend(election.create_raw_txs());

    let value_setter = value_setter_data::ValueSetterMessages {};
    messages.extend(value_setter.create_raw_txs());

    messages
}

pub fn simulate_da_with_revert_msg() -> Vec<RawTx> {
    let election = election_data::InvalidElectionCallMessages {};
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_sig() -> Vec<RawTx> {
    let election = election_data::BadSigElectionCallMessages {};
    election.create_raw_txs()
}

// TODO: Remove once we fix test with bad nonce
#[allow(unused)]
pub fn simulate_da_with_bad_nonce() -> Vec<RawTx> {
    let election = election_data::BadNonceElectionCallMessages {};
    election.create_raw_txs()
}

pub fn simulate_da_with_bad_serialization() -> Vec<RawTx> {
    let election = election_data::BadSerializationElectionCallMessages {};
    election.create_raw_txs()
}

trait MessageGenerator {
    type Call;

    fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)>;

    fn create_tx(
        &self,
        sender: MockPublicKey,
        message: Self::Call,
        nonce: u64,
        is_last: bool,
    ) -> Transaction<MockContext>;

    fn create_raw_txs(&self) -> Vec<RawTx> {
        let mut messages_iter = self.create_messages().into_iter().peekable();
        let mut serialized_messages = Vec::default();
        while let Some((sender, m, nonce)) = messages_iter.next() {
            let is_last = messages_iter.peek().is_none();

            let tx = self.create_tx(sender, m, nonce, is_last);

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
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_election_nb_of_votes_message() -> Vec<u8> {
        let query_message = election::query::QueryMessage::GenNbOfVotes;
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    pub(crate) fn generate_query_value_setter_message() -> Vec<u8> {
        let query_message = value_setter::query::QueryMessage::GetValue;
        Runtime::<MockContext>::encode_value_setter_query(query_message)
    }

    pub(crate) fn generate_query_check_balance() -> Vec<u8> {
        let query_message = sequencer::query::QueryMessage::GetSequencerAddressAndBalance;
        Runtime::<MockContext>::encode_sequencer_query(query_message)
    }
}
