#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod call;
mod error;

pub use call::CallMessage;
pub use error::RevenueShareError;
use sov_bank::utils::IntoPayable;
use sov_bank::{Amount, Coins, TokenId};
use sov_modules_api::prelude::*;
use sov_modules_api::{
    Context, CryptoSpec, Module, ModuleId, ModuleInfo, ModuleRestApi, PublicKey, Spec, StateValue,
};

/// The maximum revenue share percentage in basis points (1000 = 10%)
const MAX_REVENUE_SHARE_PERCENTAGE_IN_BASIS_POINTS: u16 = 1000;

/// The revenue share module
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct RevenueShare<S: Spec> {
    /// The unique module identifier
    #[id]
    pub id: ModuleId,

    /// Whether revenue sharing is active
    #[state]
    pub is_active: StateValue<bool>,

    /// The revenue share percentage (stored as basis points, e.g., 1000 = 10%)
    #[state]
    pub revenue_share_percentage_bps: StateValue<u16>,

    /// Override for the sovereign admin address (in case of key compromise)
    #[state]
    pub sovereign_admin_override: StateValue<S::Address>,

    /// Reference to the bank module for token transfers
    #[module]
    pub bank: sov_bank::Bank<S>,

    /// Reference to the sequencer registry module
    #[module]
    pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<S>,
}

impl<S: Spec> Module for RevenueShare<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = CallMessage<S>;
    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<Self::Spec as Spec>::Da as sov_modules_api::DaSpec>::BlockHeader,
        _config: &Self::Config,
        state: &mut impl sov_modules_api::GenesisState<S>,
    ) -> anyhow::Result<()> {
        let _ = self.is_active.set(&false, state);
        let _ = self
            .revenue_share_percentage_bps
            .set(&MAX_REVENUE_SHARE_PERCENTAGE_IN_BASIS_POINTS, state); // 10% = 1000 basis points

        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::ActivateRevenueShare => {
                self.activate_revenue_share(context, state)?;
            }
            CallMessage::DeactivateRevenueShare => {
                self.deactivate_revenue_share(context, state)?;
            }
            CallMessage::LowerRevenuePercentage {
                percentage_in_basis_points,
            } => {
                self.lower_revenue_percentage(percentage_in_basis_points, context, state)?;
            }
            CallMessage::UpdateSovereignAdmin { new_admin } => {
                self.update_sovereign_admin(new_admin, context, state)?;
            }
            CallMessage::WithdrawRewards { token_id } => {
                self.withdraw_rewards(token_id, context, state)?;
            }
        }
        Ok(())
    }
}

impl<S: Spec> RevenueShare<S> {
    /// Checks if the current transaction was processed by the "soft-confirming" /
    /// ultra-low latency preferred sequencer. If so, the revenue generated via this
    /// transaction is part of the Revenue Share terms of the Sovereign
    /// Permissionless Commercial License.
    pub fn is_preferred_sequencer(
        &self,
        context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> bool {
        let preferred_sequencer = self
            .sequencer_registry
            .get_preferred_sequencer(state)
            .ok()
            .flatten();

        if let Some((_, preferred_sequencer_address)) = preferred_sequencer {
            context.sequencer() == &preferred_sequencer_address
        } else {
            // If no preferred sequencer is set, then no sequencer is preferred.
            false
        }
    }

    /// Verifies that the caller is the sovereign admin
    fn check_if_sender_is_sov_admin(
        &self,
        context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        // Check if there's an override address first (e.g., due to key compromise)
        let admin_address = match self.sovereign_admin_override.get(state)? {
            Some(override_address) => override_address,
            None => {
                // No override, use the hardcoded admin public key from CryptoSpec
                let admin_pubkey = S::CryptoSpec::sovereign_admin_pubkey();
                // Convert public key to address via credential_id
                S::Address::from(admin_pubkey.credential_id())
            }
        };

        if context.sender() != &admin_address {
            return Err(RevenueShareError::NotAuthorized.into());
        }
        Ok(())
    }

    /// Compute and pay revenue share.
    /// This method is intended to be called by other modules that collect revenue.
    ///
    /// # Arguments
    /// * `from` - The address to pay revenue from
    /// * `token_id` - The token type being shared
    /// * `total_revenue` - The total revenue amount (the configured percentage will be taken)
    /// * `state` - The module state accessor
    pub fn compute_and_pay_revenue_share(
        &mut self,
        from: &S::Address,
        token_id: TokenId,
        total_revenue: Amount,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        let revenue_share_percentage_bps = self.get_revenue_share_percentage_bps(state);
        let revenue_share_amount = total_revenue
            .saturating_mul(Amount::from(revenue_share_percentage_bps as u128))
            .saturating_div(Amount::from(10_000u128));
        self.pay_revenue_share(from, token_id, revenue_share_amount, state)?;
        Ok(())
    }

    /// Directly pay revenue share (assuming the revenue share has already been computed
    /// by multiplying total revenue by the revenue share percentage).
    /// This method is intended to be called by other modules that collect revenue.
    ///
    /// # Arguments
    /// * `from` - The address to pay revenue from
    /// * `token_id` - The token type being shared
    /// * `amount` - The total amount to pay
    /// * `state` - The module state accessor
    pub fn pay_revenue_share(
        &mut self,
        from: &S::Address,
        token_id: TokenId,
        amount: Amount,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        // Check if revenue sharing is active
        let is_active = self.is_active.get(state)?.unwrap_or(false);
        if !is_active {
            // Revenue sharing is deactivated, do nothing
            return Ok(());
        }

        let coins = Coins { amount, token_id };

        // Transfer the revenue share to this module
        self.bank
            .transfer_from(from, self.id.to_payable(), coins, state)?;

        Ok(())
    }

    /// Get the current revenue share percentage
    pub fn get_revenue_share_percentage_bps(
        &self,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> u16 {
        self.revenue_share_percentage_bps
            .get(state)
            .ok()
            .flatten()
            .unwrap_or(MAX_REVENUE_SHARE_PERCENTAGE_IN_BASIS_POINTS)
    }

    fn activate_revenue_share(
        &mut self,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        self.check_if_sender_is_sov_admin(context, state)?;
        self.is_active.set(&true, state)?;
        Ok(())
    }

    fn deactivate_revenue_share(
        &mut self,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        self.check_if_sender_is_sov_admin(context, state)?;
        self.is_active.set(&false, state)?;
        Ok(())
    }

    fn lower_revenue_percentage(
        &mut self,
        percentage_bps: u16,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        self.check_if_sender_is_sov_admin(context, state)?;

        // Get current percentage
        let current_percentage_bps = self.get_revenue_share_percentage_bps(state);

        // Can only lower the percentage
        if percentage_bps > current_percentage_bps {
            return Err(RevenueShareError::CannotIncreasePercentage {
                current_bps: current_percentage_bps,
                new_bps: percentage_bps,
            }
            .into());
        }

        // Ensure percentage is within valid range (0-10000 basis points = 0-100%)
        if percentage_bps > MAX_REVENUE_SHARE_PERCENTAGE_IN_BASIS_POINTS {
            return Err(RevenueShareError::InvalidPercentage {
                value: percentage_bps,
            }
            .into());
        }

        self.revenue_share_percentage_bps
            .set(&percentage_bps, state)?;
        Ok(())
    }

    fn update_sovereign_admin(
        &mut self,
        new_admin: S::Address,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        self.check_if_sender_is_sov_admin(context, state)?;

        self.sovereign_admin_override.set(&new_admin, state)?;
        Ok(())
    }

    fn withdraw_rewards(
        &mut self,
        token_id: TokenId,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        self.check_if_sender_is_sov_admin(context, state)?;

        // Get the module's balance for this token
        let total_amount = self
            .bank
            .get_balance_of(self.id.to_payable(), token_id, state)?
            .unwrap_or(Amount::from(0u128));

        if total_amount == Amount::from(0u128) {
            return Err(RevenueShareError::NoRevenueToWithdraw.into());
        }

        let coins = Coins {
            amount: total_amount,
            token_id,
        };

        // Transfer all revenue to admin
        self.bank
            .transfer_from(self.id.to_payable(), context.sender(), coins, state)?;

        Ok(())
    }
}
