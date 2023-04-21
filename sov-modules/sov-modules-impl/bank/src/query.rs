use crate::{Amount, Bank};
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

/// This enumeration represents the available query messages for querying the bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    /// Gets the balance of a specified token for a specified user.
    GetBalance {
        user_address: C::Address,
        token_address: C::Address,
    },
    /// Gets the total supply of a specified token.
    GetTotalSupply { token_address: C::Address },
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct BalanceResponse {
    pub amount: Option<Amount>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct TotalSupplyResponse {
    pub amount: Option<Amount>,
}

#[rpc_gen(client, server, namespace = "bank")]
impl<C: sov_modules_api::Context> Bank<C> {
    #[rpc_method(name = "balanceOf")]
    pub(crate) fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> BalanceResponse {
        BalanceResponse {
            amount: self.get_balance_of(user_address, token_address, working_set),
        }
    }

    pub(crate) fn supply_of(
        &self,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> TotalSupplyResponse {
        TotalSupplyResponse {
            amount: self
                .tokens
                .get(&token_address, working_set)
                .map(|token| token.total_supply),
        }
    }
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub fn get_balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<u64> {
        self.tokens
            .get(&token_address, working_set)
            .and_then(|token| token.balances.get(&user_address, working_set))
    }
}
