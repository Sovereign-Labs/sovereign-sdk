use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use sov_bank::Amount;
use sov_modules_api::{HexHash, Spec};

use super::types::ExchangeRateAndGasPrice;
use crate::types::Domain;
/// Events that can be emitted by the IGP module.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Clone,
    schemars::JsonSchema,
)]
#[serde(
    bound = "S::Address: Serialize + DeserializeOwned",
    rename_all = "snake_case"
)]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum Event<S: Spec> {
    /// A complete summary of a gas payment, emitted when the igp processes a dispatched message
    ///
    /// Used by the relayer to extract payment needed for relaying.
    GasPayment {
        /// Relayer to whom payment was sent.
        relayer: S::Address,
        /// Associated message id.
        message_id: HexHash,
        /// Destination Domain of a message and payment.
        dest_domain: Domain,
        /// Gas limit for relaying
        gas_limit: Amount,
        /// Payment reward sent to the relayer, in *local* token units.
        payment: Amount,
    },
    /// Emitted when oracle data is updated for relayer and domain.
    OracleDataUpdated {
        /// Relayer.
        relayer: S::Address,
        /// Domain (chain id on hyperlane).
        domain: Domain,
        /// Oracle data
        oracle_data: ExchangeRateAndGasPrice,
    },
    /// Relayer config updated.
    RelayerConfigSet {
        /// Relayer.
        relayer: S::Address,
        /// Custom gas per domain.
        domain_custom_gas: HashMap<Domain, Amount>,
        /// Default (fallback) gas.
        default_gas: Amount,
        /// Beneficiary who can claim relayer rewards.
        beneficiary: Option<S::Address>,
    },
    /// Event emitted when beneficiary claims relayer rewards.
    RewardsClaimed {
        /// Beneficiary who claimed rewards.
        beneficiary: S::Address,
        /// Relayer whose rewards were claimed.
        relayer: S::Address,
    },
}
