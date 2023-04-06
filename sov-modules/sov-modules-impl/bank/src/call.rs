use crate::{Amount, Bank, Coins, Token};
use anyhow::{bail, Result};
use sov_modules_api::Hasher;
use sov_modules_api::{CallResponse, Spec};
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

fn prefix() -> sov_modules_api::Prefix {
    todo!()
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub fn create_token(
        &self,
        token_name: String,
        salt: u64,
        initial_balance: Amount,
        minter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let contract_address = create_contract_address::<C>(&token_name, context.sender(), salt);

        match self.tokens.get(&contract_address, working_set) {
            Some(_) => bail!("todo"),

            None => {
                let prefix = prefix();
                let balances = sov_state::StateMap::new(prefix.into());
                balances.set(&minter_address, initial_balance, working_set);

                let token = Token::<C> {
                    name: token_name,
                    total_supply: initial_balance,
                    burn_address: create_burn_address::<C>(&contract_address),
                    balances,
                };

                self.tokens.set(&contract_address, token, working_set);
            }
        };

        Ok(CallResponse::default())
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

fn create_contract_address<C: sov_modules_api::Context>(
    token_name: &str,
    sender_address: &C::Address,
    salt: u64,
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(token_name.as_bytes());
    hasher.update(sender_address.as_ref());
    hasher.update(&salt.to_le_bytes());

    let data = hasher.finalize();
    // TODO remove unwrap
    C::Address::try_from(&data).unwrap()
}

fn create_burn_address<C: sov_modules_api::Context>(contract_address: &C::Address) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(contract_address.as_ref());
    hasher.update(&[0; 32]);

    let data = hasher.finalize();
    // TODO remove unwrap
    C::Address::try_from(&data).unwrap()
}
