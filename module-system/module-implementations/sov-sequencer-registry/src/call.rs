use anyhow::{bail, Result};
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::{CallResponse, WorkingSet};

use crate::{DaAddress, SequencerRegistry};

/// This enumeration represents the available call messages for interacting with
/// the `sov-sequencer-registry` module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema),
    derive(CliWalletArg)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage {
    /// Add a new sequencer to the sequencer registry.
    Register {
        /// The DA address of the sequencer you're registering.
        da_address: DaAddress,
    },
    /// Remove a sequencer from the sequencer registry.
    Exit {
        /// The DA address of the sequencer you're removing.
        da_address: DaAddress,
    },
}

impl<C: sov_modules_api::Context> SequencerRegistry<C> {
    pub(crate) fn register(
        &self,
        da_address: Vec<u8>,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse> {
        let sequencer = context.sender();
        self.register_sequencer(da_address, sequencer, working_set)?;
        Ok(CallResponse::default())
    }

    pub(crate) fn exit(
        &self,
        da_address: Vec<u8>,
        context: &C,
        working_set: &mut WorkingSet<C>,
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

        self.delete(da_address, working_set);

        self.bank
            .transfer_from(locker, sequencer, coins, working_set)?;

        Ok(CallResponse::default())
    }

    pub(crate) fn delete(&self, da_address: Vec<u8>, working_set: &mut WorkingSet<C>) {
        self.allowed_sequencers.delete(&da_address, working_set);

        if let Some(preferred_sequencer) = self.preferred_sequencer.get(working_set) {
            if da_address == preferred_sequencer {
                self.preferred_sequencer.delete(working_set);
            }
        }
    }
}
