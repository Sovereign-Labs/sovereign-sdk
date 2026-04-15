use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_universal_wallet::UniversalWallet;

use crate::{
    capabilities::UniquenessData,
    transaction::{TransactionCallable, TxDetails},
    Spec,
};

/// V1 unsigned transaction (multisig). Constructed internally from a `Version1` transaction's
/// multisig parameters. Not typically created by consumers directly.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransactionV1<R: TransactionCallable, S: Spec> {
    /// The runtime call
    #[borsh(bound(
        serialize = "R::Call: BorshSerialize",
        deserialize = "R::Call: BorshDeserialize",
    ))]
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
    /// The multisig credential address, derived as
    /// `hash(min_signers || sorted(pub_keys))` in the rollup's native address format.
    /// Included in the signed bytes so that signers commit to the multisig configuration
    /// and prevent credential malleability from reusing signed bytes in a different
    /// multisig envelope.
    pub credential_address: S::Address,
}

impl<R: TransactionCallable, S: Spec> Clone for UnsignedTransactionV1<R, S> {
    fn clone(&self) -> Self {
        Self {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            credential_address: self.credential_address,
        }
    }
}
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransactionV1<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
            && self.credential_address == other.credential_address
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransactionV1<R, S> {}
