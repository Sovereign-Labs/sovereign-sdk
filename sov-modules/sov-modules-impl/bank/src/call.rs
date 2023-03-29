use anyhow::{ensure, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::{Amount, Bank, Coins};

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    CreateToken {
        token_name: String,
        initial_balance: Amount,

        minter_address: C::Address,
    },

    Transfer {
        to: C::Address,
        coins: Coins<C::Address>,
    },

    Burn {
        coins: Coins<C::Address>,
    },
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn create_token(
        &self,
        token_name: String,
        initial_balance: Amount,
        minter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        // This function will create a unique address for `token_name` and insert, the new `Token` to self.tokens
        todo!()
    }

    pub(crate) fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        todo!()
    }

    pub(crate) fn burn(
        &self,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        todo!()
    }
}
