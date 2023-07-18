use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::zk::{ValidityCondition, ValidityConditionChecker, Zkvm};
use sov_state::WorkingSet;

use super::AttesterIncentives;
use crate::call::Role;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: u64,
}

impl<
        C: sov_modules_api::Context,
        Vm: Zkvm,
        Cond: ValidityCondition,
        Checker: ValidityConditionChecker<Cond> + BorshDeserialize + BorshSerialize,
    > AttesterIncentives<C, Vm, Cond, Checker>
{
    /// Queries the state of the module.
    pub fn get_bond_amount(
        &self,
        address: C::Address,
        role: Role,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Response {
        match role {
            Role::Attester => {
                Response {
                    value: self
                        .bonded_attesters
                        .get(&address, working_set)
                        .unwrap_or_default(), // self.value.get(working_set),
                }
            }
            Role::Challenger => {
                Response {
                    value: self
                        .bonded_challengers
                        .get(&address, working_set)
                        .unwrap_or_default(), // self.value.get(working_set),
                }
            }
        }
    }
}
