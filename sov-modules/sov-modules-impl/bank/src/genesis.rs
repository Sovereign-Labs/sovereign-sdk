use crate::{create_token_address, token::Token, Bank};
use anyhow::{bail, Result};
use sov_state::WorkingSet;

pub const SALT: u64 = 0;
pub const SENDER: [u8; 32] = [0; 32];

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let token_address = create_token_address::<C>(&config.token_name, &SENDER, SALT);

        if self.tokens.get(&token_address, working_set).is_some() {
            // `genesis` should run on an empty db, this is just a sanity check:
            bail!("Impossible happened, token address already exists");
        }

        let token_prefix = self.prefix_from_address(&token_address);
        let balances = sov_state::StateMap::new(token_prefix);

        let mut total_supply: Option<u64> = Some(0);

        for (address, balance) in config.address_and_balances.iter() {
            balances.set(address, *balance, working_set);
            total_supply = total_supply.and_then(|ts| ts.checked_add(*balance));
        }

        let total_supply = match total_supply {
            Some(total_supply) => total_supply,
            None => bail!("Total supply overflow"),
        };

        let token = Token::<C> {
            name: config.token_name.clone(),
            total_supply,
            balances,
        };

        self.tokens.set(&token_address, token, working_set);

        Ok(())
    }
}
