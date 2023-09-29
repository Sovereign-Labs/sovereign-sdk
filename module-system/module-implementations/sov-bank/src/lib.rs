#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::*;
mod token;
/// Util functions for bank
pub mod utils;

/// Specifies the call methods using in that module.
pub use call::CallMessage;
use serde::{Deserialize, Serialize};
use sov_modules_api::{CallResponse, Error, GasUnit, ModuleInfo, WorkingSet};
use token::Token;
/// Specifies an interface to interact with tokens.
pub use token::{Amount, Coins};
/// Methods to get a token address.
pub use utils::{get_genesis_token_address, get_token_address};

/// [`TokenConfig`] specifies a configuration used when generating a token for the bank
/// module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig<C: sov_modules_api::Context> {
    /// The name of the token.
    pub token_name: String,
    /// A vector of tuples containing the initial addresses and balances (as u64)
    pub address_and_balances: Vec<(C::Address, u64)>,
    /// The addresses that are authorized to mint the token.
    pub authorized_minters: Vec<C::Address>,
    /// A salt used to encrypt the token address.
    pub salt: u64,
}

/// Initial configuration for sov-bank module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankConfig<C: sov_modules_api::Context> {
    /// A list of configurations for the initial tokens.
    pub tokens: Vec<TokenConfig<C>>,
}

/// Gas configuration for the bank module
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BankGasConfig<GU: GasUnit> {
    /// Gas price multiplier for the create token operation
    pub create_token: GU,

    /// Gas price multiplier for the transfer operation
    pub transfer: GU,

    /// Gas price multiplier for the burn operation
    pub burn: GU,

    /// Gas price multiplier for the mint operation
    pub mint: GU,

    /// Gas price multiplier for the freeze operation
    pub freeze: GU,
}

/// The sov-bank module manages user balances. It provides functionality for:
/// - Token creation.
/// - Token transfers.
/// - Token burn.
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo, Clone)]
pub struct Bank<C: sov_modules_api::Context> {
    /// The address of the sov-bank module.
    #[address]
    pub(crate) address: C::Address,

    /// The gas configuration of the sov-bank module.
    #[gas]
    pub(crate) gas: BankGasConfig<C::GasUnit>,

    /// A mapping of addresses to tokens in the sov-bank.
    #[state]
    pub(crate) tokens: sov_modules_api::StateMap<C::Address, Token<C>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Bank<C> {
    type Context = C;

    type Config = BankConfig<C>;

    type CallMessage = call::CallMessage<C>;

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::CreateToken {
                salt,
                token_name,
                initial_balance,
                minter_address,
                authorized_minters,
            } => {
                self.charge_gas(working_set, &self.gas.create_token)?;
                self.create_token(
                    token_name,
                    salt,
                    initial_balance,
                    minter_address,
                    authorized_minters,
                    context,
                    working_set,
                )?;
                Ok(CallResponse::default())
            }

            call::CallMessage::Transfer { to, coins } => {
                self.charge_gas(working_set, &self.gas.create_token)?;
                Ok(self.transfer(to, coins, context, working_set)?)
            }

            call::CallMessage::Burn { coins } => {
                self.charge_gas(working_set, &self.gas.burn)?;
                Ok(self.burn_from_eoa(coins, context, working_set)?)
            }

            call::CallMessage::Mint {
                coins,
                minter_address,
            } => {
                self.charge_gas(working_set, &self.gas.mint)?;
                self.mint_from_eoa(&coins, &minter_address, context, working_set)?;
                Ok(CallResponse::default())
            }

            call::CallMessage::Freeze { token_address } => {
                self.charge_gas(working_set, &self.gas.freeze)?;
                Ok(self.freeze(token_address, context, working_set)?)
            }
        }
    }
}
