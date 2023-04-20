use crate::{token::Token, Bank};
use anyhow::Result;
use sov_state::WorkingSet;

pub const SALT: u64 = 0;
pub const DEPLOYER: [u8; 32] = [0; 32];

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        for token_config in config.tokens.iter() {
            let (token_address, token) = Token::<C>::create(
                &token_config.token_name,
                &token_config.address_and_balances,
                &DEPLOYER,
                SALT,
                working_set,
            )?;

            // TODO: https://github.com/Sovereign-Labs/sovereign/issues/179
            // if self.tokens.get(&token_address, working_set).is_some() {
            //    bail!("Token address already exists");
            //}

            self.tokens.set(&token_address, token, working_set);
        }
        Ok(())
    }
}
