use serde::{Deserialize, Serialize};
use sov_modules_api::WorkingSet;

use super::ProverIncentives;

/// The structure containing the response returned by the `get_bond_amount` query.
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    /// The bond value stored as a `u64`.
    pub value: u64,
}

impl<C: sov_modules_api::Context, Vm: sov_modules_api::Zkvm> ProverIncentives<C, Vm> {
    /// Queries the state of the module and returns the bond amount of the address `address`.
    /// If the `address` is not bonded, returns a default value.
    pub fn get_bond_amount(
        &self,
        address: C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> Response {
        Response {
            value: self
                .bonded_provers
                .get(&address, working_set)
                .unwrap_or_default(), // self.value.get(working_set),
        }
    }
}
