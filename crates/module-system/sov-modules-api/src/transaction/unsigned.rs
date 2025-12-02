use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::SafeVec;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
#[cfg(feature = "native")]
use sov_rollup_interface::zk::CryptoSpec;

use crate::{
    capabilities::UniquenessData,
    transaction::{PriorityFeeBips, Transaction, TransactionCallable, TxDetails, Version1},
    Amount, CryptoSpecExt, Multisig, Spec,
};

/// An unsent transaction with the required data to be submitted to the DA layer
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransaction<R: TransactionCallable, S: Spec> {
    /// The runtime call
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
}
// Manually implemented to ensure correct trait bounds for the same reason as for `Transaction`
// above
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransaction<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransaction<R, S> {}

#[cfg(feature = "native")]
impl<R: TransactionCallable, S: Spec> UnsignedTransaction<R, S> {
    /// Signs the [`UnsignedTransaction`] and returns the resulting [`Transaction`].
    pub fn sign(
        self,
        private_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
    ) -> Transaction<R, S> {
        use crate::transaction::Transaction;

        Transaction::new_signed_tx(private_key, chain_hash, self)
    }
}

impl<R: TransactionCallable, S: Spec> UnsignedTransaction<R, S> {
    /// Creates a new [`UnsignedTransaction`] with the given arguments.
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

    /// Creates a new [`Transaction`] from this [`UnsignedTransaction`] when given a signature
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
