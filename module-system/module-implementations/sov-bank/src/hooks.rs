use core::str::FromStr;

use sov_modules_api::hooks::TxHooks;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, Event, Spec, StateMapAccessor, WorkingSet};

use crate::{Bank, Coins};

impl<C: Context> Bank<C> {
    fn get_address_from_events(
        &self,
        key: &str,
        working_set: &WorkingSet<C>,
    ) -> anyhow::Result<C::Address> {
        let sender_ev = Event::new(key, "");
        let sender = working_set
            .events()
            .iter()
            .find_map(|ev| (ev.key() == sender_ev.key()).then_some(ev.value().inner()))
            .ok_or_else(|| anyhow::anyhow!("failed to fetch `{}` from working set events", key))?;

        let sender = String::from_utf8(sender.clone())?;
        let sender = C::Address::from_str(&sender).map_err(|_| {
            anyhow::anyhow!(
                "failed to generate address `{}` from from event `{}`",
                key,
                sender
            )
        })?;

        Ok(sender)
    }
}

impl<C: Context> TxHooks for Bank<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<C>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        let sender = self.get_address_from_events("accounts::sender", working_set)?;
        let amount = tx.gas_limit().saturating_add(tx.gas_tip());
        let balance = self
            .tokens
            .get(&self.address, working_set)
            .and_then(|token| token.balances.get(&sender, working_set))
            .unwrap_or_default();

        if amount < balance {
            anyhow::bail!(
                " The sender `{}` lacks sufficient tokens `{}` to cover the gas cost `{}`",
                sender,
                balance,
                amount
            );
        }

        Ok(sender)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        let limit = tx.gas_limit();
        let tip = tx.gas_tip();
        let remaining = working_set.gas_remaining_funds();
        let consumed = limit.saturating_add(tip).saturating_sub(remaining);
        if consumed > 0 {
            let sender = self.get_address_from_events("accounts::sender", working_set)?;
            let coins = Coins {
                amount: consumed,
                token_address: self.address.clone(),
            };
            self.burn(coins, &sender, working_set)?;
        }

        Ok(())
    }
}
