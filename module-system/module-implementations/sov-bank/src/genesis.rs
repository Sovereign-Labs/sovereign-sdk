use anyhow::{bail, Result};
use sov_modules_api::WorkingSet;

use crate::token::Token;
use crate::Bank;

/// The address of the deployment node. For now, set to [0; 32]
pub(crate) const DEPLOYER: [u8; 32] = [0; 32];

impl<C: sov_modules_api::Context> Bank<C> {
    /// Init an instance of the bank module from the configuration `config`.
    /// For each token in the `config`, calls the [`Token::create`] function to create
    /// the token. Upon success, updates the token set if the token address doesn't already exist.
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        let parent_prefix = self.tokens.prefix();
        for token_config in config.tokens.iter() {
            let (token_address, token) = Token::<C>::create(
                &token_config.token_name,
                &token_config.address_and_balances,
                &token_config.authorized_minters,
                &DEPLOYER,
                token_config.salt,
                parent_prefix,
                working_set,
            )?;

            if self.tokens.get(&token_address, working_set).is_some() {
                bail!("Token address {} already exists", token_address);
            }

            self.tokens.set(&token_address, &token, working_set);
        }
        Ok(())
    }
}
