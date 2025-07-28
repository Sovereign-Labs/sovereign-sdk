//! This module defines abstractions and workflows around authenticating and authorizing
//! transactions within a rollup.
use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_macros::config_value_private;
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::TxHash;
use sov_state::User;
use thiserror::Error;

use crate::transaction::{
    AuthenticatedTransactionAndRawHash, Credentials, Transaction, TransactionVerificationError,
    VersionedTx,
};
use crate::{
    capabilities, metered_credential, AuthenticatedTransactionData, Context, CryptoSpec,
    DispatchCall, FullyBakedTx, GasMeter, GasMeteringError, MeteredBorshDeserialize,
    MeteredBorshDeserializeError, MeteredHasher, ProvableStateReader, RawTx, Runtime, Spec,
    StateAccessor,
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
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&tx.data).map_err(|e| {
            capabilities::fatal_deserialization_error::<_, S, _>(&tx.data, e, pre_exec_ws)
        })?;

        crate::capabilities::authenticate::<_, S, Rt>(&input.data, &Rt::CHAIN_HASH, pre_exec_ws)
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(tx: &FullyBakedTx) -> anyhow::Result<TxHash> {
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&tx.data)?;

        Ok(calculate_hash(
            &input.data,
            &mut crate::gas::UnlimitedGasMeter::<S>::default(),
        )?)
    }

    fn authenticate_unregistered<Accessor: ProvableStateReader<sov_state::User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        pre_exec_ws: &mut Accessor,
    ) -> Result<AuthenticationOutput<S, Self::Decodable>, UnregisteredAuthenticationError> {
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?;

        crate::capabilities::authenticate::<_, S, Rt>(&input.data, &Rt::CHAIN_HASH, pre_exec_ws)
            .map_err(|e| match e {
                AuthenticationError::FatalError(err, hash) => {
                    UnregisteredAuthenticationError::FatalError(err, hash)
                }
                AuthenticationError::OutOfGas(err) => {
                    UnregisteredAuthenticationError::OutOfGas(err)
                }
            })
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        AuthenticatorInput::Standard(tx)
    }
}

/// Authorizes transactions to be executed.
pub trait TransactionAuthorizer<S: Spec> {
    /// Resolves the [`Context`] for a transaction.
    fn resolve_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        sequencer_rollup_address: S::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<Context<S>>;

    /// Resolves the context for an unregistered transaction.
    fn resolve_unregistered_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<Context<S>>;

    /// Prevents duplicate transactions from running.
    fn check_uniqueness(
        &self,
        auth_data: &AuthorizationData<S>,
        context: &Context<S>,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;

    /// Marks a transaction as having been executed, preventing it from executing again.
    fn mark_tx_attempted(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;
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

/// The different types of data that can be used to verify transaction uniqueness
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UniquenessData {
    /// Nonce-based uniqueness: an account's transactions must have a unique and consecutive nonces
    Nonce(u64),
    /// Generation-based uniqueness: the last `PAST_TRANSACTION_GENERATION` generations are cached.
    /// Transactions older than this buffer are invalid, transactions falling within it or with a
    /// higher generation are valid but must have a unique hash within their generation
    Generation(u64),
}

/// Data required to authorize a sov-transaction.
pub struct AuthorizationData<S: Spec> {
    /// The nonce of the transaction.
    pub uniqueness: UniquenessData,

    /// The hash of the transaction.
    pub tx_hash: TxHash,

    /// Credential identifier used to retrieve relevant rollup address.
    pub credential_id: CredentialId,

    /// Holds the original credentials to authenticate the transaction and
    /// provides information about which `Authenticator` was used to authenticate the transaction.
    pub credentials: Credentials,

    /// The default address.
    pub default_address: S::Address,
}

fn verify_and_decode_tx<S: Spec, D: DispatchCall<Spec = S>>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    chain_hash: &[u8; 32],
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    match &tx.versioned_tx {
        VersionedTx::V0(tx_v0) => {
            if tx_v0.details.chain_id != config_chain_id() {
                return Err(AuthenticationError::FatalError(
                    FatalError::InvalidChainId {
                        expected: config_chain_id(),
                        got: tx_v0.details.chain_id,
                    },
                    raw_tx_hash,
                ));
            }

            tx.verify(chain_hash, meter).map_err(|e| match e {
                TransactionVerificationError::BadSignature(_)
                | TransactionVerificationError::TransactionDeserializationError(_) => {
                    AuthenticationError::FatalError(
                        FatalError::SigVerificationFailed(e.to_string()),
                        raw_tx_hash,
                    )
                }
                TransactionVerificationError::GasError(_) => {
                    AuthenticationError::OutOfGas(e.to_string())
                }
            })?;

            let runtime_call = tx_v0.runtime_call.clone();
            let pub_key = tx_v0.pub_key.clone();
            let credential_id = metered_credential(&pub_key, meter)
                .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

            let default_address = credential_id.into();

            let credentials = Credentials::new(pub_key);
            let generation = tx_v0.generation;

            let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
                raw_tx_hash,
                authenticated_tx: AuthenticatedTransactionData(tx_v0.details.clone()),
            };

            Ok((
                tx_and_raw_hash,
                AuthorizationData {
                    uniqueness: UniquenessData::Generation(generation),
                    tx_hash: raw_tx_hash,
                    credential_id,
                    credentials,
                    default_address,
                },
                runtime_call,
            ))
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
    let raw_tx_hash = calculate_hash::<Accessor, S>(raw_tx, state)
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
pub fn calculate_hash<G: GasMeter<Spec = S>, S: Spec>(
    data: &[u8],
    gas_meter: &mut G,
) -> Result<TxHash, GasMeteringError<S::Gas>> {
    let hash = MeteredHasher::<G, <S::CryptoSpec as CryptoSpec>::Hasher>::digest(data, gas_meter)
        .map(TxHash::new)?;

    Ok(hash)
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
    let hash = match calculate_hash::<Accessor, S>(raw_tx, pre_exec_working_set) {
        Ok(hash) => hash,
        Err(err) => return AuthenticationError::OutOfGas(err.to_string()),
    };

    AuthenticationError::FatalError(FatalError::DeserializationFailed(err.to_string()), hash)
}
