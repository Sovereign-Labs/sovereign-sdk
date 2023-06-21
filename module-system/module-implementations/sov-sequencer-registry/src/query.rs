use crate::SequencerRegistry;
use sov_modules_api::Context;
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct SequencerBalance<C: Context> {
    pub address: C::Address,
    pub balance: u64,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAndBalanceResponse<C: Context> {
    pub data: Option<SequencerBalance<C>>,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAddressResponse<C: Context> {
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context> SequencerRegistry<C> {
    /// Returns sequencer rollup address for given DA address
    /// Includes balance of sequencer in token used for staking
    #[rpc_method(name = "getSequencerAddressAndBalance")]
    pub fn sequencer_address_and_balance(
        &self,
        da_address: Vec<u8>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAndBalanceResponse<C> {
        SequencerAndBalanceResponse {
            data: self.get_seq_and_balance(da_address, working_set),
        }
    }

    /// Returns sequencer rollup address for given DA address
    /// Contains any data only if sequencer is allowed to produce batches
    #[rpc_method(name = "getSequencerAddress")]
    pub fn sequencer_address(
        &self,
        da_address: Vec<u8>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAddressResponse<C> {
        SequencerAddressResponse {
            address: self.allowed_sequencers.get(&da_address, working_set),
        }
    }

    // TODO: Do we want to list all sequencers?
}

impl<C: Context> SequencerRegistry<C> {
    fn get_seq_and_balance(
        &self,
        da_address: Vec<u8>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<SequencerBalance<C>> {
        let seq_address = self
            .allowed_sequencers
            .get_or_err(&da_address, working_set)
            .ok()?;
        let coins = self.coins_to_lock.get(working_set)?;
        let balance =
            self.bank
                .get_balance_of(seq_address.clone(), coins.token_address, working_set)?;

        Some(SequencerBalance {
            address: seq_address,
            balance,
        })
    }
}
