use crate::{Amount, Bank, Coins, Token};
use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_modules_api::Hasher;
use sov_state::WorkingSet;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    CreateToken {
        salt: u64,
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
        salt: u64,
        initial_balance: Amount,
        minter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_address = super::create_token_address::<C>(&token_name, context.sender(), salt);

        match self.tokens.get(&token_address, working_set) {
            Some(_) => bail!("todo"),

            None => {
                let prefix = self.prefix(&token_address);
                let balances = sov_state::StateMap::new(prefix);
                balances.set(&minter_address, initial_balance, working_set);

                let token = Token::<C> {
                    name: token_name,
                    total_supply: initial_balance,
                    burn_address: self.create_burn_address(&token_address),
                    balances,
                };

                self.tokens.set(&token_address, token, working_set);
            }
        };

        Ok(CallResponse::default())
    }

    pub(crate) fn transfer(
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

    pub(crate) fn burn(
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

impl<C: sov_modules_api::Context> Bank<C> {
    fn prefix(&self, token_address: &C::Address) -> sov_state::Prefix {
        let mut hasher = C::Hasher::new();
        hasher.update(self.address.as_ref());
        hasher.update(token_address.as_ref());

        //TODO address/token_address

        let hash = hasher.finalize();
        sov_state::Prefix::new(hash.to_vec())
    }

    fn create_burn_address(&self, token_address: &C::Address) -> C::Address {
        let mut hasher = C::Hasher::new();
        hasher.update(token_address.as_ref());
        hasher.update(&[0; 32]);

        let hash = hasher.finalize();
        // TODO remove unwrap
        C::Address::try_from(&hash).unwrap()
    }
}
