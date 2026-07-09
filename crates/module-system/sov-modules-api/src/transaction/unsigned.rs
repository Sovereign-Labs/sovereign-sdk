use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::SafeVec;
use sov_rollup_interface::zk::CryptoSpec;
use sov_universal_wallet::UniversalWallet;

use crate::{
    capabilities::UniquenessData,
    transaction::{
        chain_hash_fragment, PriorityFeeBips, Transaction, TransactionCallable, TxDetails,
        Version0, Version1,
    },
    Amount, CryptoSpecExt, Multisig, Spec,
};

use super::signing_payload::{
    TransactionSigningPayload, TransactionSigningPayloadV0, TransactionSigningPayloadV1,
};

/// Unsigned transaction payload. This is the consumer-facing builder type used
/// to construct transactions before signing.
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransaction<R: TransactionCallable, S: Spec> {
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
    /// Signer-declared address override.
    /// See [`crate::capabilities::AuthorizationData::address_override`] for routing semantics.
    #[serde(default)]
    pub address_override: Option<S::Address>,
}

// Manually implemented to ensure correct trait bounds (derive would require R: Clone/PartialEq)
impl<R: TransactionCallable, S: Spec> Clone for UnsignedTransaction<R, S> {
    fn clone(&self) -> Self {
        Self {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            address_override: self.address_override,
        }
    }
}
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransaction<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.uniqueness == other.uniqueness
            && self.details == other.details
            && self.address_override == other.address_override
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
    ///
    /// The transaction stores the 64-bit fragment of `chain_hash` in its details.
    pub const fn new(
        runtime_call: R::Call,
        chain_hash: [u8; 32],
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        uniqueness: UniquenessData,
        gas_limit: Option<S::Gas>,
        address_override: Option<S::Address>,
    ) -> Self {
        Self {
            runtime_call,
            uniqueness,
            details: TxDetails {
                max_priority_fee_bips,
                max_fee,
                gas_limit,
                chain_hash_fragment: chain_hash_fragment(&chain_hash),
            },
            address_override,
        }
    }

    /// Creates a new unsigned transaction with the provided metadata.
    pub const fn new_with_details(
        runtime_call: R::Call,
        uniqueness: UniquenessData,
        details: TxDetails<S>,
        address_override: Option<S::Address>,
    ) -> Self {
        Self {
            runtime_call,
            uniqueness,
            details,
            address_override,
        }
    }

    /// Creates a new [`Transaction`] from this [`UnsignedTransaction`] when given a signature
    /// and a public key.
    pub fn to_signed_tx<C: CryptoSpecExt>(
        self,
        pub_key: C::PublicKey,
        signature: C::Signature,
    ) -> Transaction<R, S, C> {
        Transaction::V0(Version0 {
            signature,
            pub_key,
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
            address_override: self.address_override,
        })
    }

    /// Creates a new `V1` transaction from this unsigned transaction.
    pub fn to_multisig_tx(
        self,
        multisig: Multisig<<S::CryptoSpec as CryptoSpec>::PublicKey>,
    ) -> Version1<R, S> {
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
            address_override: self.address_override,
        }
    }

    fn signing_payload_v0(&self, chain_hash: [u8; 32]) -> TransactionSigningPayload<R, S> {
        TransactionSigningPayload::V0(TransactionSigningPayloadV0 {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            address_override: self.address_override,
            chain_hash,
        })
    }

    pub(crate) fn signing_payload_v1_with_credential(
        &self,
        credential_address: S::Address,
        chain_hash: [u8; 32],
    ) -> TransactionSigningPayload<R, S> {
        TransactionSigningPayload::V1(TransactionSigningPayloadV1 {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            credential_address,
            address_override: self.address_override,
            chain_hash,
        })
    }

    /// Serializes the V0 transaction signing payload for this unsigned transaction.
    pub fn to_signing_bytes_v0(&self, chain_hash: [u8; 32]) -> Vec<u8> {
        self.signing_payload_v0(chain_hash).to_bytes()
    }

    /// Serializes the V1 transaction signing payload for this unsigned transaction.
    pub fn to_signing_bytes_v1(
        &self,
        multisig: &Multisig<<S::CryptoSpec as CryptoSpec>::PublicKey>,
        chain_hash: [u8; 32],
    ) -> Vec<u8> {
        let credential_address = multisig
            .credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>()
            .into();
        self.signing_payload_v1_with_credential(credential_address, chain_hash)
            .to_bytes()
    }

    /// Returns a reference to the runtime call.
    pub fn runtime_call(&self) -> &R::Call {
        &self.runtime_call
    }

    /// Returns the transaction uniqueness data.
    pub fn uniqueness(&self) -> UniquenessData {
        self.uniqueness
    }

    /// Returns the transaction details.
    pub fn details(&self) -> &TxDetails<S> {
        &self.details
    }
}
