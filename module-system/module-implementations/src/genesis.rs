use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker, Zkvm};
use sov_state::{Storage, WorkingSet};

use crate::call::Role;
use crate::AttesterIncentives;

impl<C, Vm, S, P, Cond, Checker> AttesterIncentives<C, Vm, Cond, Checker>
where
    C: sov_modules_api::Context<Storage = S>,
    Vm: Zkvm,
    S: Storage<Proof = P>,
    P: BorshDeserialize + BorshSerialize,
    Cond: ValidityCondition,
    Checker: ValidityConditionChecker<Cond>,
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

        for (attester, bond) in config.initial_attesters.iter() {
            self.bond_user_helper(*bond, attester, Role::Attester, working_set)?;
        }

        Ok(())
    }
}
