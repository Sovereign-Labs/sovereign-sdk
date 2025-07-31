//! This module defines abstractions and workflows around authenticating and authorizing
//! transactions within a rollup.
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use digest::Digest;
use serde::{Deserialize, Serialize};
use sov_modules_macros::config_value_private;
use sov_rollup_interface::TxHash;
use sov_state::User;
use thiserror::Error;

use crate::capabilities::{AuthorizationData, UniquenessData};
use crate::transaction::{
    AuthenticatedTransactionAndRawHash, Credentials, Transaction, TransactionVerificationError,
    VersionedTx,
};
use crate::{
    capabilities, metered_credential, CryptoSpec, DispatchCall, FullyBakedTx, GasMeter,
    GasMeteringError, MeteredBorshDeserialize, MeteredBorshDeserializeError, MeteredHasher,
    ProvableStateReader, RawTx, Runtime, Spec,
};

/// The chain ID of the rollup.
pub fn config_chain_id() -> u64 {
    config_value_private!("CHAIN_ID")
}

/// A batch sent by an unregistered sequencer contains only one transaction.
pub struct BatchFromUnregisteredSequencer {
    /// The transaction.
    pub tx: FullyBakedTx,
    /// Id of the batch.
    pub id: [u8; 32],
}

/// Authenticates raw transactions, ensuring that the *claimed* sender really did sign off on the transaction. Note that
/// simply *authenticating* a transaction does not guarantee that it will actually be executed. That decision is
/// made by the [`TransactionAuthorizer`]
///
/// Implementations of this trait should provide a way to interpret the raw bytes of the transaction and authenticate it.
/// Typically, the authentication will require checking the signature of the transaction.
pub trait TransactionAuthenticator<S: Spec> {
    /// The "message" that is extracted from the transaction and passed to the runtime for execution.
    type Decodable;

    /// The input to the authenticator
    type Input: BorshDeserialize + BorshSerialize + Clone + std::fmt::Debug + Send + Sync + 'static;

    /// Authenticates a transaction (typically by checking the signature) and deserializes its contents
    /// into an executable message.
    fn authenticate<Accessor: ProvableStateReader<User, Spec = S>>(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<AuthenticationOutput<S, Self::Decodable>, AuthenticationError>;

    /// MUST return the same hash as [`TransactionAuthenticator::authenticate`].
    #[cfg(feature = "native")]
    fn compute_tx_hash(tx: &FullyBakedTx) -> anyhow::Result<TxHash>;

    /// Decode a transaction into a message.
    /// This method doesn’t charge gas for deserialization, so it’s meant for off-chain code only (hence to the `native` feature).
    #[cfg(feature = "native")]
    fn decode_serialized_tx(tx: &FullyBakedTx) -> Result<Self::Decodable, FatalError>;

    /// Authenticates raw transaction that is submitted from unregistered sequencers for the
    /// purpose of forced registration (circumventing censorship by currently registered sequencers).
    ///
    /// Implementers of this method should take care to ensure that the gas consumption of this method is low because
    /// (if authentication fails) no one pays for the gas consumed by the authentication check.
    ///
    /// This is *not*  a significant DOS vector as long as gas consumption *during authentication* is reasonably low because (1)
    /// the blob storage capability bounds the number of unregistered blobs that can be submitted,
    /// and (2) if authentication succeeds then the gas for the blob is paid by the submitter.
    fn authenticate_unregistered<Accessor: ProvableStateReader<User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        state: &mut Accessor,
    ) -> Result<AuthenticationOutput<S, Self::Decodable>, UnregisteredAuthenticationError>;

    /// Encode a standard transaction for the rollup with information describing how to authenticate it.
    fn add_standard_auth(tx: RawTx) -> Self::Input;

    /// Encode the input for the authenticator into a byte array.
    #[must_use]
    fn encode_authenticator_input(input: &Self::Input) -> FullyBakedTx {
        FullyBakedTx::new(borsh::to_vec(&input).unwrap())
    }

    /// Encode a standard transaction for the rollup with information describing how to authenticate it.
    #[must_use]
    fn encode_with_standard_auth(tx: RawTx) -> FullyBakedTx {
        Self::encode_authenticator_input(&Self::add_standard_auth(tx))
    }
}

/// See [`RollupAuthenticator`].
#[derive(std::fmt::Debug, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub enum AuthenticatorInput {
    /// A rollup transaction.
    ///
    /// This is the only possible variant, but we keep this an `enum` instead of
    /// a `struct` to allow for future transaction types in a
    /// backwards-compatible way.
    Standard(RawTx),
}

/// Canonical implementation of [`TransactionAuthenticator`].
#[derive(Debug, PartialEq, Clone, Default)]
pub struct RollupAuthenticator<S, Rt>(PhantomData<(S, Rt)>);

impl<S, Rt> TransactionAuthenticator<S> for RollupAuthenticator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
{
    type Decodable = Rt::Decodable;
    type Input = AuthenticatorInput;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(tx: &FullyBakedTx) -> Result<Self::Decodable, FatalError> {
        let AuthenticatorInput::Standard(tx) = borsh::from_slice(&tx.data)
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

        capabilities::decode_sov_tx::<S, Rt>(&tx.data)
    }

    fn authenticate<Accessor: ProvableStateReader<sov_state::User, Spec = S>>(
        tx: &FullyBakedTx,
        pre_exec_ws: &mut Accessor,
    ) -> ::core::result::Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        println!("== AUTH ==");
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&tx.data).map_err(|e| {
            capabilities::fatal_deserialization_error::<_, S, _>(&tx.data, e, pre_exec_ws)
        })?;

        crate::capabilities::authenticate::<_, S, Rt>(&input.data, &Rt::CHAIN_HASH, pre_exec_ws)
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(tx: &FullyBakedTx) -> anyhow::Result<TxHash> {
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&tx.data)?;
        Ok(calculate_hash::<S>(&input.data))
    }

    fn authenticate_unregistered<Accessor: ProvableStateReader<sov_state::User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        pre_exec_ws: &mut Accessor,
    ) -> Result<AuthenticationOutput<S, Self::Decodable>, UnregisteredAuthenticationError> {
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?;

        Ok(crate::capabilities::authenticate::<_, S, Rt>(
            &input.data,
            &Rt::CHAIN_HASH,
            pre_exec_ws,
        )?)
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        AuthenticatorInput::Standard(tx)
    }
}

/// Output of the authentication.
pub type AuthenticationOutput<S, Decodable> = (
    AuthenticatedTransactionAndRawHash<S>,
    AuthorizationData<S>,
    Decodable,
);

/// Error variants that can be raised as a [`AuthenticationError::FatalError`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum FatalError {
    /// Transaction deserialization failed.
    #[error("Transaction deserialization error: {0}")]
    DeserializationFailed(String),
    /// Signature verification failed.
    #[error("Signature verification failed: {0}")]
    SigVerificationFailed(String),
    /// The chain id was invalid
    #[error("Invalid chain id: expected {expected}, got {got}")]
    InvalidChainId {
        /// The expected chain id
        expected: u64,
        /// The actual chain id
        got: u64,
    },
    /// Transaction decoding failed.
    #[error("Transaction decoding error: {0}")]
    MessageDecodingFailed(String),
    /// A variant to capture any other fatal error.
    #[error("Other: {0}")]
    Other(String),
}

/// Authentication error type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationError {
    /// The transaction authentication failed in a way that should have been detected by the sequencer before they accepted the transaction. The sequencer is slashed.
    #[error("Transaction authentication raised a fatal error, error: {0}, tx_hash {1}")]
    FatalError(FatalError, TxHash),
    /// The transaction authentication returned an error, but including it could have been an honest mistake. The sequencer should be charged enough to cover the cost of checking the transaction but not slashed.
    #[error("Transaction authentication ran out of gas: {0}.")]
    OutOfGas(
        /// The reason for the penalization.       
        String,
    ),
}

/// Authentication error relating to transactions submitted by an unregistered sequencer for the
/// purpose of direct sequencer registration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum UnregisteredAuthenticationError {
    /// The transaction authentication failed in a way that is unrecoverable.
    #[error("Transaction authentication raised a fatal error, error: {0}")]
    FatalError(FatalError, TxHash),
    /// Transaction run out of gas
    #[error("Transaction ran out of gas, error: {0}")]
    OutOfGas(String),
    /// The transaction authentication failed because the authentication discriminant was invalid.
    #[error("Invalid authentication discriminant")]
    InvalidAuthenticationDiscriminant,
}

impl From<AuthenticationError> for UnregisteredAuthenticationError {
    fn from(err: AuthenticationError) -> Self {
        match err {
            AuthenticationError::FatalError(fatal_error, hash) => {
                UnregisteredAuthenticationError::FatalError(fatal_error, hash)
            }
            AuthenticationError::OutOfGas(out_of_gas) => {
                UnregisteredAuthenticationError::OutOfGas(out_of_gas)
            }
        }
    }
}

/// Verifies that the transaction has the correct chain ID.
fn verify_chain_id<S: Spec, Call>(
    tx_v0: &crate::transaction::Version0<Call, S>,
    raw_tx_hash: TxHash,
) -> Result<(), AuthenticationError> {
    if tx_v0.details.chain_id != config_chain_id() {
        return Err(AuthenticationError::FatalError(
            FatalError::InvalidChainId {
                expected: config_chain_id(),
                got: tx_v0.details.chain_id,
            },
            raw_tx_hash,
        ));
    }
    Ok(())
}

/// Verifies the transaction signature.
fn verify_signature<S: Spec, D: DispatchCall<Spec = S>>(
    tx: &Transaction<D, S>,
    chain_hash: &[u8; 32],
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
    tx.verify(chain_hash, meter).map_err(|e| match e {
        TransactionVerificationError::GasError(_) => AuthenticationError::OutOfGas(e.to_string()),
        _ => AuthenticationError::FatalError(
            FatalError::SigVerificationFailed(e.to_string()),
            raw_tx_hash,
        ),
    })
}

/// Extracts authorization data from a verified transaction.
fn extract_authorization_data<S: Spec, D: DispatchCall<Spec = S>>(
    tx_v0: &crate::transaction::Version0<D::Decodable, S>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthorizationData<S>, AuthenticationError> {
    let pub_key = tx_v0.pub_key.clone();
    let credential_id = metered_credential(&pub_key, meter)
        .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    Ok(AuthorizationData {
        uniqueness: UniquenessData::Generation(tx_v0.generation),
        tx_hash: raw_tx_hash,
        credential_id,
        credentials: Credentials::new(pub_key),
        default_address: credential_id.into(),
    })
}

fn verify_and_decode_tx<S: Spec, D: DispatchCall<Spec = S>>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    chain_hash: &[u8; 32],
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    match &tx.versioned_tx {
        VersionedTx::V0(tx_v0) => {
            verify_chain_id(tx_v0, raw_tx_hash)?;
            verify_signature(&tx, chain_hash, raw_tx_hash, meter)?;
            let authorization_data = extract_authorization_data::<S, D>(tx_v0, raw_tx_hash, meter)?;

            let runtime_call = tx_v0.runtime_call.clone();
            let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
                raw_tx_hash,
                authenticated_tx: tx_v0.details.clone().into(),
            };

            Ok((tx_and_raw_hash, authorization_data, runtime_call))
        }
    }
}

/// Authenticate raw sov-transaction.
///
/// # Errors
/// Returns an error if gas runs out at any point, if deserialization or hashing fails, or if the
/// signature cannot be verified.
pub fn authenticate<
    Accessor: ProvableStateReader<User, Spec = S>,
    S: Spec,
    D: DispatchCall<Spec = S>,
>(
    mut raw_tx: &[u8],
    chain_hash: &[u8; 32],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    let raw_tx_hash = calculate_hash_metered::<Accessor, S>(raw_tx, state)
        .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    let tx =
        match <Transaction<D, S> as MeteredBorshDeserialize<S>>::deserialize(&mut raw_tx, state) {
            Ok(ok) => ok,

            Err(MeteredBorshDeserializeError::GasError(e)) => {
                return Err(AuthenticationError::OutOfGas(format!(
                    "Transaction deserialization run out of gas {e}, tx hash {raw_tx_hash}"
                )))
            }
            Err(MeteredBorshDeserializeError::IOError(e)) => {
                return Err(AuthenticationError::FatalError(
                    FatalError::DeserializationFailed(e.to_string()),
                    raw_tx_hash,
                ));
            }
        };

    verify_and_decode_tx::<S, D>(raw_tx_hash, tx, chain_hash, state)
}

/// Decode bytes as a Sovereign SDK transaction, returning the message and tx info.
#[cfg(feature = "native")]
pub fn decode_sov_tx<S: Spec, D: DispatchCall<Spec = S>>(
    mut raw_tx: &[u8],
) -> Result<D::Decodable, FatalError> {
    let tx = <Transaction<D, S> as MeteredBorshDeserialize<S>>::unmetered_deserialize(&mut raw_tx)
        .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    Ok(tx.call())
}

/// Calculates the hash of `data` and charges gas.
///
/// # Errors
/// Returns an error if the operation runs out of gas.
pub fn calculate_hash_metered<G: GasMeter<Spec = S>, S: Spec>(
    data: &[u8],
    gas_meter: &mut G,
) -> Result<TxHash, GasMeteringError<S::Gas>> {
    let hash = MeteredHasher::<G, <S::CryptoSpec as CryptoSpec>::Hasher>::digest(data, gas_meter)
        .map(TxHash::new)?;

    Ok(hash)
}

/// Calculates the hash of `data`.
pub fn calculate_hash<S: Spec>(data: &[u8]) -> TxHash {
    let hash = <S::CryptoSpec as CryptoSpec>::Hasher::digest(data);
    let hash_bytes: [u8; 32] = hash.into();
    TxHash::new(hash_bytes)
}

/// Helper function to create a `FatalError::DeserializationFailed` authentication error.
pub fn fatal_deserialization_error<
    Accessor: ProvableStateReader<User, Spec = S>,
    S: Spec,
    E: ToString,
>(
    raw_tx: &[u8],
    err: E,
    pre_exec_working_set: &mut Accessor,
) -> AuthenticationError {
    let hash = match calculate_hash_metered::<Accessor, S>(raw_tx, pre_exec_working_set) {
        Ok(hash) => hash,
        Err(err) => return AuthenticationError::OutOfGas(err.to_string()),
    };

    AuthenticationError::FatalError(FatalError::DeserializationFailed(err.to_string()), hash)
}
