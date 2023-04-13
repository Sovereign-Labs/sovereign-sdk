use crate::{Amount, Bank};
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

/// This enumeration represents the available query messages for querying the bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryResponse {
    /// The balance of a specified token for a specified user.
    /// This is non-optional, because all untouched accounts have a (well-defined) balance of 0.
    GetBalance { balance: u64 },
    /// The total supply of a requested token. This is optional, because because the total supply of an undefined token is undefined
    GetTotalSupply { total_supply: Option<u64> },
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct BalanceResponse {
    pub amount: Option<Amount>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Eq, PartialEq)]
pub struct TotalSupplyResponse {
    pub amount: Option<Amount>,
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn balance_of(
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
