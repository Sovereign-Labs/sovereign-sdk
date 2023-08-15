use anyhow::{bail, Result};
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::CallResponse;
use sov_rollup_interface::AddressTrait;
use sov_state::WorkingSet;

use crate::SequencerRegistry;

/// This enumeration represents the available call messages for interacting with the sov-sequencer-registry.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(CliWalletArg),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<A: AddressTrait + borsh::BorshSerialize + borsh::BorshDeserialize> {
    Register {
        // #[serde(bound(deserialize = ""))]
        da_address: A,
    },
    Exit {
        // #[serde(bound(deserialize = ""))]
        da_address: A,
    },
}

impl<
        C: sov_modules_api::Context,
        A: AddressTrait + borsh::BorshSerialize + borsh::BorshDeserialize,
    > SequencerRegistry<C, A>
{
    pub(crate) fn register(
        &self,
        da_address: &A,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let sequencer = context.sender();
        self.register_sequencer(da_address, sequencer, working_set)?;
        Ok(CallResponse::default())
    }

    pub(crate) fn exit(
        &self,
        da_address: &A,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
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

    pub(crate) fn delete(&self, da_address: &A, working_set: &mut WorkingSet<C::Storage>) {
        self.allowed_sequencers.delete(da_address, working_set);

        if let Some(preferred_sequencer) = self.preferred_sequencer.get(working_set) {
            if da_address == &preferred_sequencer {
                self.preferred_sequencer.delete(working_set);
            }
        }
    }
}
