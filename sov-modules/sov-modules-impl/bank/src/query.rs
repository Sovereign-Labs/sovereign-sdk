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
    amount: Amount,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct TotalSupplyResponse {
    amount: Amount,
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub fn balance_of(
        &self,
        _user_address: C::Address,
        _token_address: C::Address,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> BalanceResponse {
        todo!()
    }

    pub fn supply_of(
        &self,
        _user_address: C::Address,
        _token_address: C::Address,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> TotalSupplyResponse {
        todo!()
    }
}
