use core::marker::PhantomData;

use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_bank::Amount;
use sov_chain_state::TransitionHeight;
use sov_modules_api::prelude::*;
use sov_modules_api::{Context, DaSpec, ValidityConditionChecker, WorkingSet, Zkvm};
use sov_state::Storage;

use crate::{AttesterIncentives, Role};

/// Configuration of the attester incentives module
pub struct AttesterIncentivesConfig<C, Vm, Da, Checker>
where
    C: Context,
    Vm: Zkvm,
    Da: DaSpec,
    Checker: ValidityConditionChecker<Da::ValidityCondition>,
{
    /// The address of the token to be used for bonding.
    pub bonding_token_address: C::Address,
    /// The address of the account holding the reward token supply
    pub reward_token_supply_address: C::Address,
    /// The minimum bond for an attester.
    pub minimum_attester_bond: Amount,
    /// The minimum bond for a challenger.
    pub minimum_challenger_bond: Amount,
    /// A code commitment to be used for verifying proofs
    pub commitment_to_allowed_challenge_method: Vm::CodeCommitment,
    /// A list of initial provers and their bonded amount.
    pub initial_attesters: Vec<(C::Address, Amount)>,
    /// The finality period of the rollup (constant) in the number of DA layer slots processed.
    pub rollup_finality_period: TransitionHeight,
    /// The current maximum attested height
    pub maximum_attested_height: TransitionHeight,
    /// The light client finalized height
    pub light_client_finalized_height: TransitionHeight,
    /// The validity condition checker used to check validity conditions
    pub validity_condition_checker: Checker,
    /// Phantom data that contains the validity condition
    pub(crate) phantom_data: PhantomData<Da::ValidityCondition>,
}

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
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        anyhow::ensure!(
            !config.initial_attesters.is_empty(),
            "At least one prover must be set at genesis!"
        );

        self.minimum_attester_bond
            .set(&config.minimum_attester_bond, working_set);
        self.minimum_challenger_bond
            .set(&config.minimum_challenger_bond, working_set);

        self.commitment_to_allowed_challenge_method
            .set(&config.commitment_to_allowed_challenge_method, working_set);

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
