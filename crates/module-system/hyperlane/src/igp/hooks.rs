use anyhow::{anyhow, bail, Context as _, Result};
use sov_bank::{Amount, IntoPayable};
#[cfg(feature = "native")]
use sov_modules_api::prelude::tracing;
use sov_modules_api::{Context, EventEmitter, HexHash, HexString, Spec, TxState};

use super::types::RelayerWithDomainKey;
use super::{InterchainGasPaymaster, Quote};
use crate::igp::event::Event;
use crate::igp::native_gas_coins;
use crate::traits::PostDispatchHook;
use crate::types::HookType;
use crate::Message;

impl<S: Spec> PostDispatchHook<S> for InterchainGasPaymaster<S> {
    fn hook_type(
        &self,
        _addr: &S::Address,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<HookType> {
        Ok(HookType::InterchainGasPaymaster)
    }

    fn supports_metadata(
        &self,
        _metadata: &HexString,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    /// Process message, calculate gas required and validate against sent gas. On success pay to
    /// relayer. Emits `Gas payment` event.
    #[cfg_attr(feature = "native", tracing::instrument(skip(self, context, state)))]
    fn post_dispatch(
        &mut self,
        message_id: &HexHash,
        message: &Message,
        metadata: &HexString,
        relayer: &S::Address,
        gas_payment_limit: Amount,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let key = RelayerWithDomainKey::new(relayer.clone(), message.dest_domain);

        let Quote {
            metadata,
            gas_required,
        } = self
            .prepare_quote(&key, metadata, context, state)
            .context("failed preparing quote")?;

        if gas_required > gas_payment_limit {
            bail!(
                "insufficient payment to relayer. got: {}, required: {}",
                metadata.gas_limit,
                gas_required
            );
        }

        self.bank.transfer(
            self.id.to_payable(),
            native_gas_coins(gas_required),
            context,
            state,
        )?;

        let relayer_funds = self
            .funds
            .get(&key.relayer, state)
            .context("get relayer funds")?
            .unwrap_or_default();

        let relayer_funds = relayer_funds
            .checked_add(gas_required)
            .ok_or(anyhow!("relayer funds + payment overflow"))?;

        self.funds
            .set(&key.relayer, &relayer_funds, state)
            .context("set relayer funds")?;

        self.emit_event(
            state,
            Event::GasPayment {
                relayer: key.relayer,
                message_id: *message_id,
                dest_domain: message.dest_domain,
                gas_limit: metadata.gas_limit,
                payment: gas_required,
            },
        );

        Ok(())
    }

    fn quote_dispatch(
        &self,
        message: &Message,
        metadata: &HexString,
        relayer: &S::Address,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<Amount> {
        let key = RelayerWithDomainKey::new(relayer.clone(), message.dest_domain);
        let Quote { gas_required, .. } = self
            .prepare_quote(&key, metadata, context, state)
            .context("quote dispatch")?;

        Ok(gas_required)
    }
}
