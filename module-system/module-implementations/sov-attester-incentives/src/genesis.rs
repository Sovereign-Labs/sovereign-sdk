use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::ValidityConditionChecker;
use sov_state::{Storage, WorkingSet};

use crate::call::Role;
use crate::AttesterIncentives;

impl<C, Vm, S, P, Da, Checker> AttesterIncentives<C, Vm, Da, Checker>
where
    C: sov_modules_api::Context<Storage = S>,
    Vm: sov_modules_api::Zkvm,
    S: Storage<Proof = P>,
    P: BorshDeserialize + BorshSerialize,
    Da: sov_modules_api::DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        anyhow::ensure!(
            !config.initial_attesters.is_empty(),
            "At least one prover must be set at genesis!"
        );

        self.minimum_attester_bond
            .set(&config.minimum_attester_bond, working_set);
        self.minimum_challenger_bond
            .set(&config.minimum_challenger_bond, working_set);

        self.commitment_to_allowed_challenge_method.set(
            &crate::StoredCodeCommitment {
                commitment: config.commitment_to_allowed_challenge_method.clone(),
            },
            working_set,
        );

        self.rollup_finality_period
            .set(&config.rollup_finality_period, working_set);

        self.bonding_token_address
            .set(&config.bonding_token_address, working_set);

        self.reward_token_supply_address
            .set(&config.reward_token_supply_address, working_set);

        for (attester, bond) in config.initial_attesters.iter() {
            self.bond_user_helper(*bond, attester, Role::Attester, working_set)?;
        }

        self.maximum_attested_height
            .set(&config.maximum_attested_height, working_set);

        self.light_client_finalized_height
            .set(&config.light_client_finalized_height, working_set);

        self.validity_cond_checker
            .set(&config.validity_condition_checker, working_set);

        Ok(())
    }
}
