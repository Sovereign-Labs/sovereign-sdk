use anyhow::Result;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_bank::Amount;
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{GasArray, GenesisState, Module, Spec};
use sov_rollup_interface::common::SlotNumber;

use crate::ProverIncentives;

/// Configuration of the prover incentives module. Specifies the minimum bond, the commitment to
/// the allowed verifier method and a set of initial provers with their
/// bonding amount.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
#[schemars(
    bound = "S: ::sov_modules_api::Spec",
    rename = "ProverIncentivesConfig"
)]
pub struct ProverIncentivesConfig<S: Spec> {
    /// A penalty for provers who submit a proof for transitions that were already proven
    pub proving_penalty: S::Gas,
    /// The minimum bond for a prover.
    pub minimum_bond: S::Gas,
    /// A list of initial provers and their bonded amount.
    pub initial_provers: Vec<(S::Address, Amount)>,
}

impl<S: Spec> ProverIncentives<S> {
    /// Init the [`ProverIncentives`] module using the provided `config`.
    /// Sets the minimum amount necessary to bond, the commitment to the verifier circuit
    /// the bonding token ID and builds the set of initial provers.
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        anyhow::ensure!(
            !config.initial_provers.is_empty(),
            "At least one prover must be set at genesis!"
        );

        anyhow::ensure!(
            config
                .proving_penalty
                .dim_is_less_than(&config.minimum_bond),
            "The penalty should be less than the minimum bond"
        );

        self.minimum_bond.set(&config.minimum_bond, state)?;
        self.proving_penalty.set(&config.proving_penalty, state)?;
        self.last_claimed_reward.set(&SlotNumber::GENESIS, state)?;

        for (prover, bond) in config.initial_provers.iter() {
            self.register_staker(prover, prover, *bond, state)?;
        }

        Ok(())
    }
}
