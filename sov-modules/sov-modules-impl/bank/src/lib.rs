pub mod call;
mod create_token;
mod genesis;
pub mod query;
#[cfg(test)]
mod tests;
mod token;

pub use create_token::create_token_address;
use token::Token;
pub use token::{Amount, Coins};

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

/// The Bank module manages user balances. It provides functionality for:
/// - Token creation.
/// - Token transfers.
/// - Token burn.
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

    type Config = ();

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage<C>;
    type QueryResponse = query::QueryResponse;

    fn genesis(
        &self,
        _config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
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
    ) -> Self::QueryResponse {
        use query::QueryResponse;

        match msg {
            query::QueryMessage::GetBalance {
                user_address,
                token_address,
            } => {
                let response = self.balance_of(user_address, token_address, working_set);

                QueryResponse::GetBalance {
                    balance: response.amount.unwrap_or_default(),
                }
            }

            query::QueryMessage::GetTotalSupply { token_address } => {
                let response = self.supply_of(token_address, working_set);

                QueryResponse::GetTotalSupply {
                    total_supply: response.amount,
                }
            }
        }
    }
}
