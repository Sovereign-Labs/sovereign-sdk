use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::Spec;
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker, Zkvm};
use sov_state::storage::{NativeStorage, StorageKey, StorageProof};
use sov_state::{Storage, WorkingSet};

use super::AttesterIncentives;
use crate::call::Role;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct BondAmountResponse {
    pub value: u64,
}

// TODO: adapt rpc_gen macro to accept structure definitions having multiple generics
impl<C, Vm, Cond, Checker> AttesterIncentives<C, Vm, Cond, Checker>
where
    C: sov_modules_api::Context,
    Vm: Zkvm,
    Cond: ValidityCondition,
    Checker: ValidityConditionChecker<Cond> + BorshDeserialize + BorshSerialize,
{
    /// Queries the state of the module.
    pub fn get_bond_amount(
        &self,
        address: C::Address,
        role: Role,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> BondAmountResponse {
        match role {
            Role::Attester => {
                BondAmountResponse {
                    value: self
                        .bonded_attesters
                        .get(&address, working_set)
                        .unwrap_or_default(), // self.value.get(working_set),
                }
            }
            Role::Challenger => {
                BondAmountResponse {
                    value: self
                        .bonded_challengers
                        .get(&address, working_set)
                        .unwrap_or_default(), // self.value.get(working_set),
                }
            }
        }
    }

    /// Used by attesters to get a proof that they were bonded before starting to produce attestations.
    /// A bonding proof is valid for `max_finality_period` blocks, the attester can only produce transition
    /// attestations for this specific amount of time.
    pub fn get_bond_proof(
        &self,
        address: C::Address,
        witness: &<<C as Spec>::Storage as Storage>::Witness,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> StorageProof<<C::Storage as Storage>::Proof>
    where
        C::Storage: NativeStorage,
    {
        working_set.backing().get_with_proof(
            StorageKey::new(self.bonded_attesters.prefix(), &address),
            witness,
        )
    }
}
