mod data;
mod rewards;
use std::fmt::Debug;
use std::io;

use crate::Multisig;
use borsh::{BorshDeserialize, BorshSerialize};
pub use data::{AuthenticatedTransactionData, Credentials, PriorityFeeBips, TxDetails};
use derivative::Derivative;
pub(crate) use rewards::transaction_consumption_helper;
pub use rewards::{ProverReward, RemainingFunds, SequencerReward, TransactionConsumption};
#[cfg(feature = "native")]
pub use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::crypto::{SigVerificationError, Signature};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_rollup_interface::zk::CryptoSpec;
use sov_rollup_interface::TxHash;
use sov_universal_wallet::schema::UniversalWallet;
use thiserror::Error;
pub use types::{
    v0::Version0,
    v1::{PubKeyAndSignature, Version1},
};
pub use unsigned::UnsignedTransaction;

use crate::capabilities::UniquenessData;
use crate::{
    CryptoSpecExt, DispatchCall, Gas, GasMeter, GasMeteringError, GasSpec, MeteredBorshDeserialize,
    MeteredBorshDeserializeError, MeteredSigVerificationError, MeteredSignature, Spec,
};

#[cfg(test)]
mod tests;
mod types;
mod unsigned;

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
    derive_more::Debug, // derive_more uses the correct bound of TransactionCallable::RuntimeCall
    Clone,
    Derivative,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    UniversalWallet,
)]
#[derivative(
    PartialEq(bound = "R::Call: PartialEq + Eq"),
    Eq(bound = "R::Call: PartialEq + Eq")
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
/// A Transaction object that is compatible with the module-system/sov-default-stf.
pub enum Transaction<R: TransactionCallable, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// V0 Transaction type.
    V0(
        #[borsh(bound(
            serialize = "<C as CryptoSpec>::Signature: BorshSerialize, <C as CryptoSpec>::PublicKey: BorshSerialize",
            deserialize = "<C as CryptoSpec>::Signature: BorshDeserialize, <C as CryptoSpec>::PublicKey: BorshDeserialize",
        ))]
        Version0<R::Call, S, C>,
    ),
    /// A V1 (multisig) transaction.
    V1(
        #[borsh(bound(
            serialize = "<C as CryptoSpec>::Signature: BorshSerialize, <C as CryptoSpec>::PublicKey: BorshSerialize",
            deserialize = "<C as CryptoSpec>::Signature: BorshDeserialize, <C as CryptoSpec>::PublicKey: BorshDeserialize",
        ))]
        Version1<R::Call, S, C>,
    ),
}

#[cfg(feature = "native")]
impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Transaction<R, S, C> {
    /// Computes the transaction hash as it would be computed by the rollup.
    /// We do it so by computing the hash of the borsh-serialized transaction.
    pub fn hash(&self) -> TxHash {
        use digest::Digest;

        let data = borsh::to_vec(&self).unwrap();
        <S::CryptoSpec as CryptoSpec>::Hasher::digest(&data).into()
    }

    /// Creates a new signed transaction using the provided private key.
    pub fn new_signed_tx(
        priv_key: &C::PrivateKey,
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

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Transaction<R, S, C> {
    /// Convenience function to return the transaction bytes in the correct format ready for submission.
    pub fn tx_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("Serialization should be never fail")
    }

    fn unmetered_deserialize_inner(buf: &mut &[u8]) -> Result<Self, io::Error> {
        let this = <Transaction<R, S, C> as borsh::BorshDeserialize>::deserialize(buf)?;
        tracing::trace!(transaction = ?this, "Deserialized transaction");
        Ok(this)
    }
}

/// Charge gas for deserializing a transaction.
pub fn charge_tx_deserialization<S: Spec>(
    meter: &mut impl GasMeter<Spec = S>,
    len: usize,
) -> Result<(), MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
    crate::charge_gas_to_deserialize(
        S::tx_bias_borsh_deserialization(),
        S::tx_gas_to_charge_per_byte_borsh_deserialization(),
        len,
        meter,
    )
}

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> MeteredBorshDeserialize<S>
    for Transaction<R, S, C>
{
    #[cfg_attr(feature = "bench", crate::cycle_tracker)]
    #[cfg_attr(
        all(feature = "gas-constant-estimation", feature = "native"),
        crate::track_gas_constants_usage
    )]
    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        charge_tx_deserialization(meter, buf.len())?;

        Transaction::<R, S, C>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    #[cfg(feature = "native")]
    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        Transaction::<R, S, C>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }
}

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

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Transaction<R, S, C> {
    /// Returns a reference to the runtime call of the transaction.
    pub fn runtime_call(&self) -> &R::Call {
        match &self {
            Transaction::V0(inner) => &inner.runtime_call,
            Transaction::V1(inner) => &inner.runtime_call,
        }
    }

    /// Extract the runtime call from the transaction
    pub fn into_runtime_call(self) -> R::Call {
        match self {
            Transaction::V0(inner) => inner.runtime_call,
            Transaction::V1(inner) => inner.runtime_call,
        }
    }

    /// Returns the chain id.
    pub fn chain_id(&self) -> u64 {
        match &self {
            Transaction::V0(inner) => inner.details.chain_id,
            Transaction::V1(inner) => inner.details.chain_id,
        }
    }

    /// Creates a new transaction with the provided metadata.
    pub fn new_with_details_v0(
        pub_key: C::PublicKey,
        runtime_call: R::Call,
        signature: C::Signature,
        uniqueness: UniquenessData,
        details: TxDetails<S>,
    ) -> Self {
        Self::V0(Version0 {
            signature,
            pub_key,
            runtime_call,
            uniqueness,
            details,
        })
    }

    /// Serialize the transaction, appending the runtime's chain_hash.
    /// This is the standard serialization for Sovereign signature signing.
    pub fn serialized_with_chain_hash(
        &self,
        chain_hash: &[u8; 32],
    ) -> Result<Vec<u8>, TransactionVerificationError<S::Gas>> {
        let mut serialized_tx = borsh::to_vec(&self.to_unsigned_transaction()).map_err(|e| {
            TransactionVerificationError::TransactionDeserializationError(e.to_string())
        })?;
        serialized_tx.extend_from_slice(chain_hash);
        Ok(serialized_tx)
    }

    /// Charge gas for verifying the transaction signature against the given message.
    pub fn charge_gas_for_signature(
        &self,
        msg_len: usize,
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<(), TransactionVerificationError<S::Gas>> {
        match &self {
            Transaction::V0(inner) => {
                MeteredSignature::new::<S>(inner.signature.clone()).charge_gas(meter, msg_len)?;
            }
            Transaction::V1(inner) => {
                for signature in inner.signatures.iter() {
                    // Charge gas for all the signatures up front before verifying. This way, we can switch to batch verification and the gas price will be the same.
                    MeteredSignature::new::<S>(signature.signature.clone())
                        .charge_gas(meter, msg_len)?;
                }
            }
        }
        Ok(())
    }

    /// Verify the transaction signature against the provided message.
    /// *Does not* charge gas for verification. Callers are responsible for calling
    /// `charge_gas_for_signature()` with the same message for gas metering.
    pub fn verify_signature_unmetered(
        &self,
        msg: &[u8],
    ) -> Result<(), TransactionVerificationError<S::Gas>> {
        match &self {
            Transaction::V0(inner) => {
                inner
                    .signature
                    .verify(&inner.pub_key, msg)
                    .map_err(TransactionVerificationError::BadSignature)?;
            }
            Transaction::V1(inner) => {
                let all_signers = inner
                    .signatures
                    .iter()
                    .map(|s| s.pub_key.clone())
                    .chain(inner.unused_pub_keys.iter().cloned())
                    .collect::<Vec<_>>();
                let multisig = Multisig::new(inner.min_signers, all_signers);
                multisig
                    .verify_signature(msg, &inner.signatures)
                    .map_err(TransactionVerificationError::BadSignature)?;
            }
        }
        Ok(())
    }

    /// Converts the transaction to an unsigned transaction.
    pub fn to_unsigned_transaction(&self) -> UnsignedTransaction<R, S> {
        match &self {
            Transaction::V0(inner) => UnsignedTransaction::new_with_details(
                inner.runtime_call.clone(),
                inner.uniqueness,
                inner.details.clone(),
            ),
            Transaction::V1(inner) => UnsignedTransaction::new_with_details(
                inner.runtime_call.clone(),
                inner.uniqueness,
                inner.details.clone(),
            ),
        }
    }
}

/// A struct containing an authenticated transaction and its associated hash.
pub struct AuthenticatedTransactionAndRawHash<S: Spec> {
    /// Hash of raw bytes.
    pub raw_tx_hash: TxHash,
    /// Authenticated transaction data.
    pub authenticated_tx: AuthenticatedTransactionData<S>,
}

pub(crate) mod hex_field_format {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S, V>(value: &V, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: AsRef<[u8]>,
    {
        let s = hex::encode(value.as_ref());
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>>,
    {
        let hex_string = String::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_string).map_err(serde::de::Error::custom)?;
        T::try_from(bytes).map_err(|_e| serde::de::Error::custom("invalid hex"))
    }
}
