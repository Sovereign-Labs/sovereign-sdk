mod call;
mod genesis;
mod query;
#[cfg(test)]
mod tests;
mod token;
use sov_modules_api::Error;
use sov_modules_api::Hasher;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;
pub use token::{Amount, Coins, Token};

#[derive(ModuleInfo)]
pub struct Bank<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub tokens: sov_state::StateMap<C::Address, Token<C>>,
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

fn create_token_address<C: sov_modules_api::Context>(
    token_name: &str,
    sender_address: &C::Address,
    salt: u64,
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(sender_address.as_ref());
    hasher.update(token_name.as_bytes());
    hasher.update(&salt.to_le_bytes());

    let hash = hasher.finalize();
    // TODO remove unwrap
    C::Address::try_from(&hash).unwrap()
}
