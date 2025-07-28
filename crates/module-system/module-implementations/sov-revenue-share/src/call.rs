use schemars::JsonSchema;
use sov_bank::TokenId;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::Spec;

/// Call messages for the revenue share module
#[derive(Debug, Clone, PartialEq, JsonSchema, UniversalWallet, Eq)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Activate revenue sharing (admin only)
    ActivateRevenueShare,

    /// Deactivate revenue sharing (admin only)
    DeactivateRevenueShare,

    /// Lower the revenue share percentage (admin only)
    LowerRevenuePercentage {
        /// New percentage in basis points (e.g., 1000 = 10%)
        percentage_in_basis_points: u16,
    },

    /// Update the sovereign admin address (current admin only)
    UpdateSovereignAdmin {
        /// The new admin address
        new_admin: S::Address,
    },

    /// Withdraw accumulated rewards to the admin address (admin only)
    WithdrawRewards {
        /// The token ID to withdraw
        token_id: TokenId,
    },
}
