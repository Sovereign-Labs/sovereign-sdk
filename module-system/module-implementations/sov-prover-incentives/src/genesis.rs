use anyhow::Result;
use sov_modules_api::WorkingSet;

use crate::ProverIncentives;

impl<C: sov_modules_api::Context, Vm: sov_modules_api::Zkvm> ProverIncentives<C, Vm> {
    /// Init the [`ProverIncentives`] module using the provided `config`.
    /// Sets the minimum amount necessary to bond, the commitment to the verifier circuit
    /// the bonding token address and builds the set of initial provers.
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        anyhow::ensure!(
            !config.initial_provers.is_empty(),
            "At least one prover must be set at genesis!"
        );

        self.minimum_bond.set(&config.minimum_bond, working_set);
        self.commitment_of_allowed_verifier_method
            .set(&config.commitment_of_allowed_verifier_method, working_set);
        self.bonding_token_address
            .set(&config.bonding_token_address, working_set);

        for (prover, bond) in config.initial_provers.iter() {
            self.bond_prover_helper(*bond, prover, working_set)?;
        }

        Ok(())
    }
}
