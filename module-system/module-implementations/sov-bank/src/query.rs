use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

use crate::{Amount, Bank};

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BalanceResponse {
    pub amount: Option<Amount>,
}

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TotalSupplyResponse {
    pub amount: Option<Amount>,
}

#[rpc_gen(client, server, namespace = "bank")]
impl<C: sov_modules_api::Context> Bank<C> {
    #[rpc_method(name = "balanceOf")]
    pub fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> BalanceResponse {
        BalanceResponse {
            amount: self.get_balance_of(user_address, token_address, working_set),
        }
    }

    #[rpc_method(name = "supplyOf")]
    pub fn supply_of(
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
