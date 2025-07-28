#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod call;
mod capability;
pub mod derived_holder;
#[cfg(feature = "test-utils")]
mod test_utils;

pub use capability::ReserveGasError;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::*;
mod token;
/// Util functions for bank
pub mod utils;
pub use call::*;
pub use genesis::*;
use sov_modules_api::macros::config_value;
pub use sov_modules_api::Amount;
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateMap,
    TxState,
};
use sov_state::BorshCodec;
use token::{BalanceKey, Token};
/// Specifies an interface to interact with tokens.
pub use token::{BurnRate, Coins, TokenId, TokenIdBech32};
/// Methods to get a token ID.
pub use utils::{get_token_id, IntoPayable, Payable};
use utils::{TokenHolder, TokenHolderRef};

/// Event definition from module exported
/// This can be useful for deserialization from API and similar cases
pub mod event;
use crate::event::Event;

/// The default decimals value that will be used for newly created tokens unless specified
/// otherwise in the callmessage.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 8;

/// The [`TokenId`] of the rollup's gas token.
pub fn config_gas_token_id() -> TokenId {
    config_value!("GAS_TOKEN_ID")
}

pub(crate) type C = BorshCodec;

/// The sov-bank module manages user balances. It provides functionality for:
/// - Token creation.
/// - Token transfers.
/// - Token burn.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct Bank<S: Spec> {
    /// The id of the sov-bank module.
    #[id]
    pub id: ModuleId,

    /// A mapping of [`TokenId`]s to tokens in the sov-bank.
    #[state]
    pub(crate) tokens: StateMap<TokenId, Token<S>, C>,

    /// A mapping from [`TokenHolder`] and[`TokenId`] to balance in the sov-bank.
    #[state]
    pub(crate) balances: StateMap<BalanceKey<TokenHolder<S>>, Amount, C>,
}

impl<S: Spec> Module for Bank<S> {
    type Spec = S;

    type Config = BankConfig<S>;

    type CallMessage = call::CallMessage<S>;

    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            call::CallMessage::CreateToken {
                token_name,
                token_decimals,
                initial_balance,
                mint_to_address,
                supply_cap,
                admins,
            } => {
                let admins = admins
                    .iter()
                    .map(|minter| TokenHolderRef::from(&minter))
                    .collect::<Vec<_>>();

                self.create_token(
                    token_name.into(),
                    token_decimals,
                    initial_balance,
                    &mint_to_address,
                    admins,
                    supply_cap,
                    context.sender(),
                    state,
                )?;
                Ok(())
            }

            call::CallMessage::Transfer { to, coins } => {
                Ok(self.transfer(&to, coins, context, state)?)
            }
            call::CallMessage::Burn { coins } => Ok(self.burn_from_eoa(coins, context, state)?),
            call::CallMessage::Mint {
                coins,
                mint_to_address,
            } => {
                self.mint_from_eoa(coins, &mint_to_address, context, state)?;
                Ok(())
            }
            call::CallMessage::Freeze { token_id } => Ok(self.freeze(token_id, context, state)?),
            call::CallMessage::UpdateAdmin {
                new_admin,
                token_id,
            } => {
                self.update_admin_address(new_admin, token_id, context, state)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn custom_gas_token_id() {
        env::set_var(
            "SOV_TEST_CONST_OVERRIDE_GAS_TOKEN_ID",
            "token_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqnfxkwm",
        );
        assert_eq!(
            config_gas_token_id().to_string(),
            "token_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqnfxkwm"
        );
    }
}
