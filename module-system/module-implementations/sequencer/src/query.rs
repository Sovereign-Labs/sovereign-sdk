#[cfg(feature = "native")]
use crate::Sequencer;
use sov_modules_api::AddressBech32;
#[cfg(feature = "native")]
use sov_modules_api::Context;

#[cfg(feature = "native")]
use sov_state::WorkingSet;

/// This enumeration represents the available query messages for querying the sequencer module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetSequencerAddressAndBalance,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct Data {
    pub address: AddressBech32,
    pub balance: u64,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAndBalanceResponse {
    pub data: Option<Data>,
}

#[cfg(feature = "native")]
impl<C: Context> Sequencer<C> {
    pub(crate) fn sequencer_address_and_balance(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAndBalanceResponse {
        SequencerAndBalanceResponse {
            data: self.get_seq_and_balance(working_set),
        }
    }
}

#[cfg(feature = "native")]
impl<C: Context> Sequencer<C> {
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
