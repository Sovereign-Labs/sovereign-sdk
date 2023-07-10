use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::traits::Zkvm;
use sov_state::{Storage, WorkingSet};

use crate::{call::Role, AttesterIncentives};

impl<C, Vm: Zkvm, S, P> AttesterIncentives<C, Vm>
where
    C: sov_modules_api::Context<Storage = S>,
    S: Storage<Proof = P>,
    P: BorshDeserialize + BorshSerialize,
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

        self.bonding_token_address
            .set(&config.bonding_token_address, working_set);

        for (attester, bond) in config.initial_attesters.iter() {
            self.bond_user_helper(*bond, attester, Role::Attester, working_set)?;
        }

        Ok(())
    }
}
