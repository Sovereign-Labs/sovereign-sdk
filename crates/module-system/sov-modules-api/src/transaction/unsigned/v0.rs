use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::SafeVec;
use sov_rollup_interface::zk::CryptoSpec;
use sov_universal_wallet::UniversalWallet;

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
    #[borsh(bound(
        serialize = "R::Call: BorshSerialize",
        deserialize = "R::Call: BorshDeserialize",
    ))]
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
}

// Manually implemented to ensure correct trait bounds (derive would require R: Clone/PartialEq)
impl<R: TransactionCallable, S: Spec> Clone for UnsignedTransactionV0<R, S> {
    fn clone(&self) -> Self {
        Self {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
        }
    }
}
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransactionV0<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransactionV0<R, S> {}

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
