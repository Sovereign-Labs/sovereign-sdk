use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::SafeVec;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_rollup_interface::zk::CryptoSpec;

use crate::{
    capabilities::UniquenessData,
    transaction::{PriorityFeeBips, Transaction, TransactionCallable, TxDetails, Version1},
    Amount, CryptoSpecExt, Multisig, Spec,
};

/// V0 unsigned transaction (single-sig). This is the consumer-facing builder type used
/// to construct transactions before signing.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransactionV0<R: TransactionCallable, S: Spec> {
    /// The runtime call
    #[borsh(bound(serialize = "", deserialize = "",))]
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
}

// Manually implemented to ensure correct trait bounds
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransactionV0<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransactionV0<R, S> {}

/// V1 unsigned transaction (multisig). Constructed internally from a `Version1` transaction's
/// multisig parameters. Not typically created by consumers directly.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransactionV1<R: TransactionCallable, S: Spec> {
    /// The runtime call
    #[borsh(bound(serialize = "", deserialize = "",))]
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

impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransactionV1<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
            && self.credential_address == other.credential_address
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransactionV1<R, S> {}

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
    V0(#[borsh(bound(serialize = "", deserialize = "",))] UnsignedTransactionV0<R, S>),
    /// V1 (multisig) unsigned transaction, includes credential commitment.
    V1(#[borsh(bound(serialize = "", deserialize = "",))] UnsignedTransactionV1<R, S>),
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

    /// Returns a clone of the runtime call.
    pub fn call(&self) -> R::Call {
        self.runtime_call().clone()
    }
}

#[cfg(feature = "native")]
impl<R: TransactionCallable, S: Spec> UnsignedTransactionV0<R, S> {
    /// Signs the [`UnsignedTransactionV0`] and returns the resulting [`Transaction`].
    pub fn sign(
        self,
        private_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
    ) -> Transaction<R, S> {
        use crate::transaction::Transaction;

        Transaction::new_signed_tx(private_key, chain_hash, self)
    }
}

impl<R: TransactionCallable, S: Spec> UnsignedTransactionV0<R, S> {
    /// Creates a new [`UnsignedTransactionV0`] with the given arguments.
    pub const fn new(
        runtime_call: R::Call,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        uniqueness: UniquenessData,
        gas_limit: Option<S::Gas>,
    ) -> Self {
        Self {
            runtime_call,
            uniqueness,
            details: TxDetails {
                max_priority_fee_bips,
                max_fee,
                gas_limit,
                chain_id,
            },
        }
    }

    /// Creates a new unsigned transaction with the provided metadata.
    pub const fn new_with_details(
        runtime_call: R::Call,
        uniqueness: UniquenessData,
        details: TxDetails<S>,
    ) -> Self {
        Self {
            runtime_call,
            uniqueness,
            details,
        }
    }

    /// Creates a new [`Transaction`] from this [`UnsignedTransactionV0`] when given a signature
    /// and a public key.
    pub fn to_signed_tx<C: CryptoSpecExt>(
        self,
        pub_key: C::PublicKey,
        signature: C::Signature,
    ) -> Transaction<R, S, C> {
        Transaction::new_with_details_v0(
            pub_key,
            self.runtime_call,
            signature,
            self.uniqueness,
            self.details,
        )
    }

    /// Creates a new `V1` transaction from this unsigned transaction.
    pub fn to_multisig_tx(
        self,
        multisig: Multisig<<S::CryptoSpec as CryptoSpec>::PublicKey>,
    ) -> Version1<R::Call, S> {
        Version1 {
            signatures: SafeVec::new(),
            unused_pub_keys: multisig
                .signers
                .try_into()
                .expect("Too many signers in multisig"),
            min_signers: multisig.required_signers,
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
        }
    }

    /// Returns a copy of the RuntimeCall from this unsigned transaction.
    pub fn call(&self) -> R::Call {
        self.runtime_call.clone()
    }
}
