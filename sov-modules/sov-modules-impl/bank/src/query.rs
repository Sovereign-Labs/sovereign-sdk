use sov_state::WorkingSet;

use crate::{Amount, Bank};

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetBalance {
        user_address: C::Address,
        token_address: C::Address,
    },

    GetTotalSupply {
        token_address: C::Address,
    },
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct BalanceResponse {
    amount: Option<Amount>,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct TotalSupplyResponse {
    amount: Option<Amount>,
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> BalanceResponse {
        BalanceResponse {
            amount: self
                .tokens
                .get(&token_address, working_set)
                .and_then(|token| token.balances.get(&user_address, working_set)),
        }
    }

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
