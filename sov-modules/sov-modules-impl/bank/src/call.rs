use anyhow::Result;
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
    pub fn create_token(
        &self,
        _token_name: String,
        _initial_balance: Amount,
        _minter_address: C::Address,
        _context: &C,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        // This function will create a unique address for `token_name` and insert, the new `Token` to self.tokens
        todo!()
    }

    pub fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_address = coins.token_address;

        let token = self.tokens.get_or_err(&token_address, working_set)?;
        token.transfer(context.sender(), &to, coins.amount, working_set)
    }

    pub fn burn(
        &self,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_address = coins.token_address;
        let token = self.tokens.get_or_err(&token_address, working_set)?;

        token.burn(context.sender(), coins.amount, working_set)
    }
}
