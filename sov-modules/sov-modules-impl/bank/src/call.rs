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
        _to: C::Address,
        _coins: Coins<C::Address>,
        _context: &C,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        todo!()
    }

    pub fn burn(
        &self,
        _coins: Coins<C::Address>,
        _context: &C,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        todo!()
    }
}
