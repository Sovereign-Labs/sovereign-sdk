use anyhow::bail;
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::{CallResponse, WorkingSet};

use crate::SequencerRegistry;

/// This enumeration represents the available call messages for interacting with
/// the `sov-sequencer-registry` module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema),
    derive(CliWalletArg)
)]
#[derive(Debug, PartialEq, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub enum CallMessage {
    /// Add a new sequencer to the sequencer registry.
    Register {
        /// The raw Da address of the sequencer you're registering.
        da_address: Vec<u8>,
    },
    /// Remove a sequencer from the sequencer registry.
    Exit {
        /// The raw Da address of the sequencer you're removing.
        da_address: Vec<u8>,
    },
}

impl<C: sov_modules_api::Context, Da: sov_modules_api::DaSpec> SequencerRegistry<C, Da> {
    pub(crate) fn register(
        &self,
        da_address: &Da::Address,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse> {
        let sequencer = context.sender();
        self.register_sequencer(da_address, sequencer, working_set)?;
        Ok(CallResponse::default())
    }

    pub(crate) fn exit(
        &self,
        da_address: &Da::Address,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<CallResponse> {
        let locker = &self.address;
        let coins = self.coins_to_lock.get_or_err(working_set)?;
        let sequencer = context.sender();

        let belongs_to = self
            .allowed_sequencers
            .get_or_err(da_address, working_set)?;

        if sequencer != &belongs_to {
            bail!("Unauthorized exit attempt");
        }

        self.delete(da_address, working_set);

        self.bank
            .transfer_from(locker, sequencer, coins, working_set)?;

        Ok(CallResponse::default())
    }

    pub(crate) fn delete(&self, da_address: &Da::Address, working_set: &mut WorkingSet<C>) {
        self.allowed_sequencers.delete(da_address, working_set);

        if let Some(preferred_sequencer) = self.preferred_sequencer.get(working_set) {
            if da_address == &preferred_sequencer {
                self.preferred_sequencer.delete(working_set);
            }
        }
    }
}
