use sov_modules_api::AddressBech32;
use sov_state::WorkingSet;

use crate::Sequencer;

/// This enumeration represents the available query messages for querying the sequencer module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetSequencerAddressAndBalance,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct Data {
    pub address: AddressBech32,
    pub balance: u64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct SequencerAndBalanceResponse {
    pub data: Option<Data>,
}

impl<C: sov_modules_api::Context> Sequencer<C> {
    pub(crate) fn sequencer_address_and_balance(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAndBalanceResponse {
        SequencerAndBalanceResponse {
            data: self.get_seq_and_balance(working_set),
        }
    }
}

impl<C: sov_modules_api::Context> Sequencer<C> {
    fn get_seq_and_balance(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<Data> {
        let seq_address = self.seq_rollup_address.get(working_set)?;
        let coins = self.coins_to_lock.get(working_set)?;
        let balance =
            self.bank
                .get_balance_of(seq_address.clone(), coins.token_address, working_set)?;

        Some(Data {
            address: seq_address.into(),
            balance,
        })
    }
}
