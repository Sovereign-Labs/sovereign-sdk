mod call;
mod create_token;
mod genesis;
mod query;
#[cfg(test)]
mod tests;
mod token;

pub use create_token::create_token_address;
use token::Token;
pub use token::{Amount, Coins};

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

///
#[derive(ModuleInfo)]
pub struct Bank<C: sov_modules_api::Context> {
    /// The address of the bank module.
    #[address]
    pub(crate) address: C::Address,

    /// A mapping of addresses to tokens in the bank.
    #[state]
    pub(crate) tokens: sov_state::StateMap<C::Address, Token<C>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Bank<C> {
    type Context = C;

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage<C>;

    fn genesis(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<(), Error> {
        Ok(self.init_module(working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::CreateToken {
                salt,
                token_name,
                initial_balance,
                minter_address,
            } => Ok(self.create_token(
                token_name,
                salt,
                initial_balance,
                minter_address,
                context,
                working_set,
            )?),

            call::CallMessage::Transfer { to, coins } => {
                Ok(self.transfer(to, coins, context, working_set)?)
            }

            call::CallMessage::Burn { coins } => Ok(self.burn(coins, context, working_set)?),
        }
    }

    #[cfg(feature = "native")]
    fn query(
        &self,
        msg: Self::QueryMessage,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> sov_modules_api::QueryResponse {
        match msg {
            query::QueryMessage::GetBalance {
                user_address,
                token_address,
            } => {
                let response =
                    serde_json::to_vec(&self.balance_of(user_address, token_address, working_set))
                        .unwrap();

                sov_modules_api::QueryResponse { response }
            }

            query::QueryMessage::GetTotalSupply { token_address } => {
                let response =
                    serde_json::to_vec(&self.supply_of(token_address, working_set)).unwrap();

                sov_modules_api::QueryResponse { response }
            }
        }
    }
}
