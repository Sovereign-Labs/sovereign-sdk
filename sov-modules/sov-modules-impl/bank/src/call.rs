use anyhow::{ensure, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::{Amount, Bank, Coins};

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    CreateToken {
        // Q: should `token_name` be unique or should we allow multiple tokens with the same name.
        token_name: String,
        initial_balance: Amount,
        // Q: should we allow only the sender of the tx to be the minter?
        minter_address: C::Address,
    },

    Transfer {
        to: C::Address,
        coins: Coins<C::Address>,
    },

    Burn {
        coins: Coins<C::Address>,
    },
    // We don't have "Mint" message (the initial supply is set with `CreateToken` message)
    // We can add it later or maybe we should crate a new module "Minter/Staker".
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
