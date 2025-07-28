use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_bank::Amount;
use sov_modules_api::{GenesisState, Module, Spec};
use sov_rollup_interface::common::SlotNumber;

use crate::AttesterIncentives;

/// Configuration of the attester incentives module
#[derive(Debug, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct AttesterIncentivesConfig<S: Spec> {
    /// The minimum bond for an attester.
    pub minimum_attester_bond: S::Gas,
    /// The minimum bond for a challenger.
    pub minimum_challenger_bond: S::Gas,
    /// A list of initial attesters and their bonded amount.
    pub initial_attesters: Vec<(S::Address, Amount)>,
    /// The finality period of the rollup (constant) in the number of DA layer slots processed.
    pub rollup_finality_period: SlotNumber, // TODO: use a newtype Delta<SlotNumber>
    /// The current maximum attested height
    pub maximum_attested_height: SlotNumber,
    /// The light client finalized height
    pub light_client_finalized_height: SlotNumber,
}

impl<S: Spec> AttesterIncentives<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        anyhow::ensure!(
            !config.initial_attesters.is_empty(),
            "At least one prover must be set at genesis!"
        );

        self.minimum_attester_bond
            .set(&config.minimum_attester_bond, state)?;
        self.minimum_challenger_bond
            .set(&config.minimum_challenger_bond, state)?;

        self.rollup_finality_period
            .set(&config.rollup_finality_period, state)?;

        for (attester, bond) in config.initial_attesters.iter() {
            self.register_attester(*bond, attester, state)?;
        }

        self.maximum_attested_height
            .set(&config.maximum_attested_height, state)?;

        self.light_client_finalized_height
            .set(&config.light_client_finalized_height, state)?;

        Ok(())
    }
}
