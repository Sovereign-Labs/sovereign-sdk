mod data;
mod rewards;
use std::fmt::Debug;
use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
pub use data::{AuthenticatedTransactionData, Credentials, PriorityFeeBips, TxDetails};
pub(crate) use rewards::transaction_consumption_helper;
pub use rewards::{ProverReward, RemainingFunds, SequencerReward, TransactionConsumption};
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
pub use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::crypto::SigVerificationError;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_rollup_interface::zk::CryptoSpec;
use sov_rollup_interface::TxHash;
use sov_universal_wallet::schema::UniversalWallet;
use thiserror::Error;

use crate::{
    Amount, DispatchCall, Gas, GasMeter, GasMeteringError, GasSpec, MeteredBorshDeserialize,
    MeteredBorshDeserializeError, MeteredSigVerificationError, MeteredSignature, Spec,
};

#[cfg(test)]
mod tests;

/// Structures that implement this trait represent a call message that can be included in a
/// transaction.
/// By default, this is blanket-derived on anything implementing the `Runtime` trait, implementing
/// the trait using the runtime's typed `RuntimeCall` messages.
pub trait TransactionCallable {
    /// The type of the call of the transaction.
    type Call: BorshSerialize + BorshDeserialize + Debug + Clone + PartialEq + Eq + UniversalWallet;
}

impl<D: DispatchCall> TransactionCallable for D {
    type Call = D::Decodable;
}

#[derive(
    derive_more::Debug,
    Clone,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[serde(bound = "Call: serde::Serialize + serde::de::DeserializeOwned")]
/// V0 transaction.
pub struct Version0<Call, S: Spec> {
    /// The signature of the transaction.
    pub signature: <S::CryptoSpec as CryptoSpec>::Signature,
    /// The public key of the sender of the transaction.
    pub pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
    /// The runtime call of the transaction.
    #[sov_wallet(
        bound = "Call: sov_rollup_interface::sov_universal_wallet::schema::UniversalWallet"
    )]
    pub runtime_call: Call,
    /// The generation of the transaction (for uniqueness).
    pub generation: u64,
    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
}

#[derive(
    derive_more::Debug,
    Clone,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[serde(bound = "Call: serde::Serialize + serde::de::DeserializeOwned")]
/// Versioned transaction
pub enum VersionedTx<Call, S: Spec> {
    /// V0 transaction.
    V0(Version0<Call, S>),
}

#[derive(
    derive_more::Debug, // derive_more uses the correct bound of TransactionCallable::RuntimeCall
    Clone,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    UniversalWallet,
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
/// A Transaction object that is compatible with the module-system/sov-default-stf.
pub struct Transaction<R: TransactionCallable, S: Spec> {
    /// Versioned transaction.
    pub versioned_tx: VersionedTx<R::Call, S>,
}

#[cfg(feature = "native")]
impl<R: TransactionCallable, S: Spec> Transaction<R, S> {
    /// Computes the transaction hash as it would be computed by the rollup.
    /// We do it so by computing the hash of the borsh-serialized transaction.
    pub fn hash(&self) -> TxHash {
        use digest::Digest;

        let data = borsh::to_vec(&self).unwrap();
        <S::CryptoSpec as CryptoSpec>::Hasher::digest(&data).into()
    }
}

impl<R: TransactionCallable, S: Spec> Transaction<R, S> {
    fn unmetered_deserialize_inner(buf: &mut &[u8]) -> Result<Self, io::Error> {
        let this = <Transaction<R, S> as borsh::BorshDeserialize>::deserialize(buf)?;
        tracing::trace!(transaction = ?this, "Deserialized transaction");
        Ok(this)
    }
}

impl<R: TransactionCallable, S: Spec> MeteredBorshDeserialize<S> for Transaction<R, S> {
    fn bias_borsh_deserialization() -> <S as Spec>::Gas {
        S::tx_bias_borsh_deserialization()
    }

    fn gas_to_charge_per_byte_borsh_deserialization() -> <S as Spec>::Gas {
        S::tx_gas_to_charge_per_byte_borsh_deserialization()
    }

    #[cfg_attr(feature = "bench", crate::cycle_tracker)]
    #[cfg_attr(
        all(feature = "gas-constant-estimation", feature = "native"),
        crate::track_gas_constants_usage
    )]
    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        Self::charge_gas_to_deserialize(buf, meter)?;

        Transaction::<R, S>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    #[cfg(feature = "native")]
    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        Transaction::<R, S>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }
}

// Unfortunately built-in Rust derives for Eq and PartialEq use a bound of `TransactionCallable:
// Eq, PartialEq` which is incorrect (TransactionCallable will be the Runtime in most cases).
// Thus we have to manually derive them, because the real bound is
// `TransactionCallable::RuntimeCall: Eq` (aka `<Runtime as DispatchCall>::Decodable`) which is already
// enforced in the trait definition.
impl<R: TransactionCallable, S: Spec> PartialEq for Transaction<R, S> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.versioned_tx, &other.versioned_tx) {
            (VersionedTx::V0(self_inner), VersionedTx::V0(other_inner)) => {
                self_inner.signature == other_inner.signature
                    && self_inner.pub_key == other_inner.pub_key
                    && self_inner.runtime_call == other_inner.runtime_call
                    && self_inner.generation == other_inner.generation
                    && self_inner.details == other_inner.details
            }
        }
    }
}
impl<R: TransactionCallable, S: Spec> Eq for Transaction<R, S> {}

/// Errors that can be raised by the [`Transaction::verify`] method.
#[derive(Error, Debug)]
pub enum TransactionVerificationError<GU: Gas> {
    /// An error occurred when deserializing the transaction.
    #[error("Impossible to deserialize transaction: {0}")]
    TransactionDeserializationError(String),
    /// The signature check failed.
    #[error("Invalid signature: {0}")]
    BadSignature(SigVerificationError),
    /// There is not enough gas to verify the signature.
    #[error("A gas error was raised when trying to verify the signature, {0}")]
    GasError(GasMeteringError<GU>),
}

impl<GU: Gas> From<MeteredSigVerificationError<GU>> for TransactionVerificationError<GU> {
    fn from(value: MeteredSigVerificationError<GU>) -> TransactionVerificationError<GU> {
        match value {
            MeteredSigVerificationError::BadSignature(err) => {
                TransactionVerificationError::BadSignature(err)
            }
            MeteredSigVerificationError::GasError(err) => {
                TransactionVerificationError::GasError(err)
            }
        }
    }
}

impl<R: TransactionCallable, S: Spec> Transaction<R, S> {
    /// Returns a reference to the runtime call of the transaction.
    pub fn runtime_call(&self) -> &R::Call {
        match &self.versioned_tx {
            VersionedTx::V0(inner) => &inner.runtime_call,
        }
    }

    /// Returns the chain id.
    pub fn chain_id(&self) -> u64 {
        match &self.versioned_tx {
            VersionedTx::V0(inner) => inner.details.chain_id,
        }
    }

    /// Check whether the transaction has been signed correctly.
    ///
    /// # Errors
    /// Returns an error if:
    ///  * The signature is wrong
    ///  * Serializing or hashing the transaction fails
    ///  * Any operation runs out of gas
    pub fn verify(
        &self,
        chain_hash: &[u8; 32],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<(), TransactionVerificationError<S::Gas>> {
        let mut serialized_tx = borsh::to_vec(&self.to_unsigned_transaction()).map_err(|e| {
            TransactionVerificationError::TransactionDeserializationError(e.to_string())
        })?;
        serialized_tx.extend_from_slice(chain_hash);

        match &self.versioned_tx {
            VersionedTx::V0(inner) => {
                MeteredSignature::new::<S>(inner.signature.clone())
                    .verify(&inner.pub_key, &serialized_tx, meter)
                    .map_err(TransactionVerificationError::from)?;
            }
        }
        Ok(())
    }

    /// Creates a new transaction with the provided metadata.
    pub fn new_with_details_v0(
        pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        runtime_call: R::Call,
        signature: <S::CryptoSpec as CryptoSpec>::Signature,
        generation: u64,
        details: TxDetails<S>,
    ) -> Self {
        Self {
            versioned_tx: VersionedTx::V0(Version0 {
                signature,
                pub_key,
                runtime_call,
                generation,
                details,
            }),
        }
    }

    /// Extract the runtime call from the transaction
    pub fn call(self) -> R::Call {
        match self.versioned_tx {
            VersionedTx::V0(inner) => inner.runtime_call.clone(),
        }
    }

    fn to_unsigned_transaction(&self) -> UnsignedTransaction<R, S> {
        match &self.versioned_tx {
            VersionedTx::V0(inner) => UnsignedTransaction::new_with_details(
                inner.runtime_call.clone(),
                inner.generation,
                inner.details.clone(),
            ),
        }
    }
}

#[cfg(feature = "native")]
impl<R: TransactionCallable, S: Spec> Transaction<R, S> {
    /// New signed transaction.
    pub fn new_signed_tx(
        priv_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
        unsigned_tx: UnsignedTransaction<R, S>,
    ) -> Self {
        let mut utx_bytes: Vec<u8> = Vec::new();
        BorshSerialize::serialize(&unsigned_tx, &mut utx_bytes).unwrap();
        utx_bytes.extend_from_slice(chain_hash);

        let pub_key = priv_key.pub_key();
        let signature = priv_key.sign(&utx_bytes);

        unsigned_tx.to_signed_tx(pub_key, signature)
    }
}

/// An unsent transaction with the required data to be submitted to the DA layer
#[derive(
    derive_more::Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, UniversalWallet,
)]
pub struct UnsignedTransaction<R: TransactionCallable, S: Spec> {
    // The runtime call
    runtime_call: R::Call,
    // The generation number
    generation: u64,
    // Data related to fees and gas handling.
    details: TxDetails<S>,
}
// Manually implemented to ensure correct trait bounds for the same reason as for `Transaction`
// above
impl<R: TransactionCallable, S: Spec> PartialEq for UnsignedTransaction<R, S> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime_call == other.runtime_call
            && self.generation == other.generation
            && self.details == other.details
    }
}
impl<R: TransactionCallable, S: Spec> Eq for UnsignedTransaction<R, S> {}

impl<R: TransactionCallable, S: Spec> UnsignedTransaction<R, S> {
    /// Creates a new [`UnsignedTransaction`] with the given arguments.
    pub const fn new(
        runtime_call: R::Call,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        generation: u64,
        gas_limit: Option<S::Gas>,
    ) -> Self {
        Self {
            runtime_call,
            generation,
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
        generation: u64,
        details: TxDetails<S>,
    ) -> Self {
        Self {
            runtime_call,
            generation,
            details,
        }
    }

    /// Creates a new [`Transaction`] from this [`UnsignedTransaction`] when given a signature
    /// and a public key.
    pub fn to_signed_tx(
        self,
        pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        signature: <S::CryptoSpec as CryptoSpec>::Signature,
    ) -> Transaction<R, S> {
        Transaction::new_with_details_v0(
            pub_key,
            self.runtime_call,
            signature,
            self.generation,
            self.details,
        )
    }
}

/// A struct containing an authenticated transaction and its associated hash.
pub struct AuthenticatedTransactionAndRawHash<S: Spec> {
    /// Hash of raw bytes.
    pub raw_tx_hash: TxHash,
    /// Authenticated transaction data.
    pub authenticated_tx: AuthenticatedTransactionData<S>,
}
