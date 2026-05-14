use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_universal_wallet::UniversalWallet;

use crate::{
    capabilities::UniquenessData,
    transaction::{TransactionCallable, TxDetails},
    Spec,
};

/// V0 transaction signing payload. This is the Borsh payload signed by single-sig
/// transactions.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct TransactionSigningPayloadV0<R: TransactionCallable, S: Spec> {
    /// The runtime call
    #[borsh(bound(
        serialize = "R::Call: BorshSerialize",
        deserialize = "R::Call: BorshDeserialize",
    ))]
    pub(crate) runtime_call: R::Call,
    /// The uniqueness identifier
    pub(crate) uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub(crate) details: TxDetails<S>,
    /// Signer-declared address override.
    /// See [`crate::capabilities::AuthorizationData::address_override`] for routing semantics.
    #[serde(default)]
    pub(crate) address_override: Option<S::Address>,
    /// The chain hash binding this signature to the rollup schema and metadata.
    #[sov_wallet(hidden)]
    pub(crate) chain_hash: [u8; 32],
}

impl<R: TransactionCallable, S: Spec> Clone for TransactionSigningPayloadV0<R, S> {
    fn clone(&self) -> Self {
        Self {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            address_override: self.address_override,
            chain_hash: self.chain_hash,
        }
    }
}

impl<R: TransactionCallable, S: Spec> PartialEq for TransactionSigningPayloadV0<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
            && self.address_override == other.address_override
            && self.chain_hash == other.chain_hash
    }
}

impl<R: TransactionCallable, S: Spec> Eq for TransactionSigningPayloadV0<R, S> {}
