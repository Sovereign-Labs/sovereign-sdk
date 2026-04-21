pub(crate) mod v0;
pub(crate) mod v1;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_universal_wallet::UniversalWallet;

pub use v0::UnsignedTransactionV0;
pub use v1::UnsignedTransactionV1;

use crate::{
    capabilities::UniquenessData,
    transaction::{TransactionCallable, TxDetails},
    Spec,
};

/// Versioned unsigned transaction. The borsh enum discriminant serves as the version byte
/// in signed bytes: `V0` = `0x00`, `V1` = `0x01`.
///
/// This is the type serialized (with the chain hash appended) to produce the bytes that
/// are signed and verified. The enum discriminant ensures V0 and V1 signed bytes are
/// distinct, preventing cross-version replay attacks.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub enum UnsignedTransaction<R: TransactionCallable, S: Spec> {
    /// V0 (single-sig) unsigned transaction.
    V0(
        #[borsh(bound(
            serialize = "R::Call: BorshSerialize",
            deserialize = "R::Call: BorshDeserialize",
        ))]
        UnsignedTransactionV0<R, S>,
    ),
    /// V1 (multisig) unsigned transaction, includes credential commitment.
    V1(
        #[borsh(bound(
            serialize = "R::Call: BorshSerialize",
            deserialize = "R::Call: BorshDeserialize",
        ))]
        UnsignedTransactionV1<R, S>,
    ),
}

impl<R: TransactionCallable, S: Spec> Clone for UnsignedTransaction<R, S> {
    fn clone(&self) -> Self {
        match self {
            Self::V0(v0) => Self::V0(v0.clone()),
            Self::V1(v1) => Self::V1(v1.clone()),
        }
    }
}
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransaction<R, S> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::V0(a), Self::V0(b)) => a == b,
            (Self::V1(a), Self::V1(b)) => a == b,
            _ => false,
        }
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransaction<R, S> {}

// Getters on the enum for version-agnostic field access
impl<R: TransactionCallable, S: Spec> UnsignedTransaction<R, S> {
    /// Serializes the versioned unsigned transaction and appends the chain hash,
    /// producing the bytes used for signing and signature verification.
    pub fn serialized_with_chain_hash(&self, chain_hash: &[u8; 32]) -> Vec<u8> {
        let mut serialized_tx = borsh::to_vec(self).expect("Serialization to vec is infallible");
        serialized_tx.extend_from_slice(chain_hash);
        serialized_tx
    }

    /// Returns a reference to the runtime call.
    pub fn runtime_call(&self) -> &R::Call {
        match self {
            Self::V0(v0) => &v0.runtime_call,
            Self::V1(v1) => &v1.runtime_call,
        }
    }

    /// Returns the uniqueness data.
    pub fn uniqueness(&self) -> UniquenessData {
        match self {
            Self::V0(v0) => v0.uniqueness,
            Self::V1(v1) => v1.uniqueness,
        }
    }

    /// Returns a reference to the transaction details.
    pub fn details(&self) -> &TxDetails<S> {
        match self {
            Self::V0(v0) => &v0.details,
            Self::V1(v1) => &v1.details,
        }
    }
}
