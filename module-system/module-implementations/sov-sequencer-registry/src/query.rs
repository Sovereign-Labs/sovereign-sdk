use crate::SequencerRegistry;
use sov_modules_api::AddressBech32;
use sov_modules_api::Context;
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct Data {
    // TODO: Do we know, that there's default address implementation and might not work for all DAs
    pub address: AddressBech32,
    pub balance: u64,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAndBalanceResponse {
    pub data: Option<Data>,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub struct SequencerAddressResponse<C: Context> {
    pub address: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "sequencer")]
impl<C: Context> SequencerRegistry<C> {
    #[rpc_method(name = "getSequencerAddressAndBalance")]
    pub fn sequencer_address_and_balance(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> SequencerAndBalanceResponse {
        SequencerAndBalanceResponse {
            data: self.get_seq_and_balance(working_set),
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
