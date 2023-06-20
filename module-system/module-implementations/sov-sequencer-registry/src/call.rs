use crate::SequencerRegistry;
use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

/// This enumeration represents the available call messages for interacting with the sov-sequencer-registry.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage {
    Register { da_address: Vec<u8> },
    // TODO: Allow to exit funds to another address?
    Exit { da_address: Vec<u8> },
}

impl<C: sov_modules_api::Context> SequencerRegistry<C> {
    pub(crate) fn register(
        &self,
        da_address: Vec<u8>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let sequencer = context.sender();
        if self
            .allowed_sequencers
            .get(&da_address, working_set)
            .is_some()
        {
            bail!("sequencer {} already registered", sequencer)
        }

        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;
        self.bank
            .transfer_from(sequencer, locker, coins, working_set)?;

        self.allowed_sequencers
            .set(&da_address, sequencer, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn exit(
        &self,
        da_address: Vec<u8>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;
        let sequencer = context.sender();

        let belongs_to = self
            .allowed_sequencers
            .get_or_err(&da_address, working_set)?;

        if sequencer != &belongs_to {
            bail!("Unauthorized exit attempt");
        }

        self.allowed_sequencers.delete(&da_address, working_set);

        self.bank
            .transfer_from(locker, sequencer, coins, working_set)?;

        Ok(CallResponse::default())
    }

    fn slash(&self, da_address: Vec<u8>, working_set: &mut WorkingSet<C::Storage>) {
        self.allowed_sequencers.delete(&da_address, working_set)
    }
}
