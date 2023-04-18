use sov_state::WorkingSet;

use crate::Sequencer;

/// This enumeration represents the available query messages for querying the sequencer module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetSequencerAddressAndBalance,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct SequencerAndBalanceResponse {
    // TODO: Add address after: https://github.com/Sovereign-Labs/sovereign/pull/166 is merged
    // address: Option<C::Address>,
    pub amount: Option<u64>,
}

impl<C: sov_modules_api::Context> Sequencer<C> {
    pub(crate) fn sequencer_address_and_balance(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAndBalanceResponse {
        // TODO add seq address
        SequencerAndBalanceResponse {
            amount: self.get_seq_and_balance(working_set).map(|res| res.1),
        }
    }
}

impl<C: sov_modules_api::Context> Sequencer<C> {
    fn get_seq_and_balance(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<(C::Address, u64)> {
        let seq_address = self.seq_rollup_address.get(working_set)?;
        let coins = self.coins_to_lock.get(working_set)?;
        let balance =
            self.bank
                .get_balance_of(seq_address.clone(), coins.token_address, working_set)?;

        Some((seq_address, balance))
    }
}
