//! Defines rpc queries exposed by the bank module, along with the relevant types
use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::WorkingSet;

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
        version: Option<u64>,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<BalanceResponse> {
        if let Some(v) = version {
            working_set.set_archival_version(v)
        }
        Ok(BalanceResponse {
            amount: self.get_balance_of(user_address, token_address, working_set),
        })
    }

    #[rpc_method(name = "supplyOf")]
    /// Rpc method that returns the supply of a token stored at the address `token_address`.
    pub fn supply_of(
        &self,
        version: Option<u64>,
        token_address: C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<TotalSupplyResponse> {
        if let Some(v) = version {
            working_set.set_archival_version(v)
        }
        Ok(TotalSupplyResponse {
            amount: self.get_total_supply_of(&token_address, working_set),
        })
    }
}
