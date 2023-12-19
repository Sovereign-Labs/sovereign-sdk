use core::str::FromStr;
use std::rc::Rc;

use sov_modules_api::hooks::TxHooks;
use sov_modules_api::macros::config_constant;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, WorkingSet};

use crate::{Bank, Coins};

#[config_constant]
// This constant is a fixed value, expected to be generated as
//
// ```rust
// let token_name = "sov-gas-token";
// let deployer = DEPLOYER;
// let salt = 0;
// let computed = super::get_token_address::<DefaultContext>(token_name, &deployer, salt);
// ```
//
// TODO: fetch address as constant
// https://github.com/Sovereign-Labs/sovereign-sdk/issues/1234
const GAS_TOKEN_ADDRESS: &'static str;

/// The computed addresses of a pre-dispatch tx hook.
pub struct BankTxHook<C: Context> {
    /// The tx sender address
    pub sender: Rc<C::Address>,
    /// The sequencer address
    pub sequencer: Rc<C::Address>,
}

impl<C: Context> TxHooks for Bank<C> {
    type Context = C;
    type PreArg = BankTxHook<C>;
    type PreResult = ();

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<C>,
        working_set: &mut WorkingSet<C>,
        hook: BankTxHook<C>,
    ) -> anyhow::Result<()> {
        let BankTxHook { sender, sequencer } = hook;
        let amount = tx.gas_limit().saturating_add(tx.gas_tip());

        if amount > 0 {
            let token_address = C::Address::from_str(GAS_TOKEN_ADDRESS)?;
            let from = &sender;
            let to = &sequencer;
            let coins = Coins {
                amount,
                token_address,
            };
            self.transfer_from(from, to, coins, working_set)?;
        }

        Ok(())
    }

    fn post_dispatch_tx_hook(
        &self,
        _tx: &Transaction<Self::Context>,
        ctx: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        let amount = working_set.gas_remaining_funds();

        if amount > 0 {
            let token_address = C::Address::from_str(GAS_TOKEN_ADDRESS)?;
            let from = ctx.sequencer();
            let to = ctx.sender();
            let coins = Coins {
                amount,
                token_address,
            };
            self.transfer_from(from, to, coins, working_set)?;
        }

        Ok(())
    }
}
