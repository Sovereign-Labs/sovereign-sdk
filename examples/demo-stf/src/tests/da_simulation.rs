use std::rc::Rc;

use borsh::BorshSerialize;
use sov_data_generators::data_gen::MessageGenerator;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::PublicKey;
use sov_modules_stf_template::RawTx;

use crate::runtime::Runtime;

pub fn simulate_da(
    value_setter_admin: DefaultPrivateKey,
    election_admin: DefaultPrivateKey,
) -> Vec<RawTx> {
    let mut messages = Vec::default();

    let election = election_data::ElectionCallMessages::new(election_admin);
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
