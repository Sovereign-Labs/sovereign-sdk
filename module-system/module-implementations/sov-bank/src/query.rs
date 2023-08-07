#![allow(missing_docs)]
use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;

use crate::{Amount, Bank};

/// Structure returned by the `balance_of` rpc method.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct BalanceResponse {
    /// The balance amount of a given user for a given token. Equivalent to u64.
    pub amount: Option<Amount>,
}

/// Structure returned by the `supply_of` rpc method.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct TotalSupplyResponse {
    /// The amount of token supply for a given token address. Equivalent to u64.
    pub amount: Option<Amount>,
}

#[rpc_gen(client, server, namespace = "bank")]
impl<C: sov_modules_api::Context> Bank<C> {
    #[rpc_method(name = "balanceOf")]
    /// Rpc method that returns the balance of the user at the address `user_address` for the token
    /// stored at the address `token_address`.
    pub fn balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<BalanceResponse> {
        Ok(BalanceResponse {
            amount: self.get_balance_of(user_address, token_address, working_set),
        })
    }

    #[rpc_method(name = "supplyOf")]
    /// Rpc method that returns the supply of token of the token stored at the address `token_address`.
    pub fn supply_of(
        &self,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<TotalSupplyResponse> {
        Ok(TotalSupplyResponse {
            amount: self
                .tokens
                .get(&token_address, working_set)
                .map(|token| token.total_supply),
        })
    }
}

impl<C: sov_modules_api::Context> Bank<C> {
    /// Helper function used by the rpc method [`balance_of`] to return the balance of the token stored at `token_address`
    /// for the user having the address `user_address` from the underlying storage. If the token address doesn't exist, or
    /// if the user doesn't have tokens of that type, return `None`. Otherwise, wrap the resulting balance in `Some`.
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
