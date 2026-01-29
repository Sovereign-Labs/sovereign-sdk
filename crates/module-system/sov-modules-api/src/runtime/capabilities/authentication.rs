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

use crate::capabilities::AuthorizationData;
use crate::transaction::{
    AuthenticatedTransactionAndRawHash, Transaction, TransactionVerificationError, TxDetails,
};
#[cfg(feature = "native")]
use crate::CryptoSpecExt;
use crate::GetGasPrice;
use crate::{
    capabilities, CryptoSpec, DispatchCall, FullyBakedTx, GasMeter, GasMeteringError,
    MeteredBorshDeserialize, MeteredBorshDeserializeError, MeteredHasher, ProvableStateReader,
    RawTx, Runtime, Spec, VersionReader,
};

/// The chain ID of the rollup.
pub fn config_chain_id() -> u64 {
    config_value_private!("CHAIN_ID")
}

/// Resolves all valid chain hashes for a given height using configured overrides.
///
/// This function loads the `CHAIN_HASH_OVERRIDES` from config and uses them
/// to resolve the appropriate chain hashes for the given height, including
/// any hashes that are valid due to grace periods.
///
/// This is useful for endpoints (like `/rollup/schema`) that need to return
/// the currently active chain hash for wallets.
pub fn resolve_chain_hashes_for_height(
    height: u64,
    default_hash: [u8; 32],
) -> crate::runtime::ResolvedChainHashes {
    let overrides: &[crate::ChainHashOverride] = &config_value_private!("CHAIN_HASH_OVERRIDES");
    crate::runtime::resolve_chain_hashes(height, overrides, default_hash)
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
    type Decodable: Send;

    /// The input to the authenticator
    type Input: BorshDeserialize + BorshSerialize + Clone + std::fmt::Debug + Send + Sync + 'static;

    /// Authenticates a transaction (typically by checking the signature) and deserializes its contents
    /// into an executable message.
    /// For rollups running a preferred sequencer it is expected that implementations cache signature
    /// checks during native execution, and the preferred sequencer will attempt to pre-populate this
    /// cache to parallelise signature checks.
    fn authenticate<
        Accessor: ProvableStateReader<User, Spec = S>
            + GetGasPrice<Spec = S>
            + crate::StateMetricsProvider
            + VersionReader,
    >(
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
    fn authenticate_unregistered<
        Accessor: ProvableStateReader<User, Spec = S>
            + GetGasPrice<Spec = S>
            + crate::StateMetricsProvider
            + VersionReader,
    >(
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

#[cfg(feature = "native")]
/// A helper type intended for caching the results of signature verification outside of the ZKVM.
/// In particular, the preferred sequencer expects a cache to be available and attempts to warm it
/// upon receiving a transaction, prior to executing it inside the STF.
///
/// # Type Parameter
/// - `T`: The success type of the verification result. Can be `()` if only success/failure is
///   stored.
///
/// # Usage Example
/// ```rust,ignore
/// #[cfg(feature = "native")]
/// static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<()>> =
///     std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));
///
/// fn verify_signature(...) -> Result<(), AuthenticationError> {
///     #[cfg(feature = "native")]
///     if let Some(cached) = SIGNATURE_CACHE.get(&tx_hash) {
///         return cached;
///     }
///
///     let result = /* actual verification */;
///
///     #[cfg(feature = "native")]
///     SIGNATURE_CACHE.insert(tx_hash, result.clone());
///
///     result
/// }
/// ```
pub type SignatureVerificationCache<T, TxId = TxHash> =
    quick_cache::sync::Cache<TxId, Result<T, AuthenticationError>>;

#[cfg(feature = "native")]
/// The default size for signature verification caches.
///
/// At 5k TPS, this gives us 50 seconds of cache lifetime. This should be long enough that
/// signatures almost always last until the node has processed the block.
pub const DEFAULT_SIGNATURE_CACHE_SIZE: usize = 250_000;

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

#[cfg(feature = "native")]
static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<(), (TxHash, [u8; 32])>> =
    std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));

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

    fn authenticate<Accessor: ProvableStateReader<sov_state::User, Spec = S> + VersionReader>(
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
        Ok(calculate_hash::<S>(&input.data))
    }

    fn authenticate_unregistered<
        Accessor: ProvableStateReader<sov_state::User, Spec = S>
            + crate::GetGasPrice<Spec = S>
            + crate::StateMetricsProvider
            + VersionReader,
    >(
        batch: &BatchFromUnregisteredSequencer,
        pre_exec_ws: &mut Accessor,
    ) -> Result<AuthenticationOutput<S, Self::Decodable>, UnregisteredAuthenticationError> {
        let AuthenticatorInput::Standard(input) = borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?;

        authenticate_unregistered::<Accessor, S, Rt>(&input.data, pre_exec_ws)
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
    /// Missing chain id
    #[error("Missing chain id: expected {0}")]
    MissingChainId(u64),
    /// The chain id was invalid
    #[error("Invalid chain id: expected {expected}, got {got}")]
    InvalidChainId {
        /// The expected chain id
        expected: u64,
        /// The actual chain id
        got: u64,
    },
    /// The chain hash was invalid. Not every type of transaction will be able to throw this (some
    /// will just implicitly fail signature checks).
    #[error("Invalid chain hash: expected {expected}, got {got}")]
    InvalidChainHash {
        /// The expected chain id
        expected: String,
        /// The actual chain id
        got: String,
    },
    /// The chain name was invalid. Not every type of transaction will be able to throw this (some
    /// will just implicitly fail signature checks).
    #[error("Invalid chain name: expected {expected}, got {got}")]
    InvalidChainName {
        /// The expected chain id
        expected: String,
        /// The actual chain id
        got: String,
    },
    /// Transaction decoding failed.
    #[error("Transaction decoding error: {0}")]
    MessageDecodingFailed(String),
    /// The user's max_fee_per_gas is insufficient to cover rollup's base fee.
    #[error("Insufficient max_fee_per_gas: user specified {user_max_fee_per_gas}, but current base fee is {rollup_base_fee}")]
    InsufficientMaxFeePerGas {
        /// The user's max_fee_per_gas in wei
        user_max_fee_per_gas: u128,
        /// The rollup's current base fee in wei
        rollup_base_fee: u128,
    },
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
pub fn verify_chain_id<S: Spec>(
    tx_details: &TxDetails<S>,
    raw_tx_hash: TxHash,
) -> Result<(), AuthenticationError> {
    if tx_details.chain_id != config_chain_id() {
        return Err(AuthenticationError::FatalError(
            FatalError::InvalidChainId {
                expected: config_chain_id(),
                got: tx_details.chain_id,
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
    let serialized_tx = tx.serialized_with_chain_hash(chain_hash).map_err(|e| {
        AuthenticationError::FatalError(
            FatalError::DeserializationFailed(e.to_string()),
            raw_tx_hash,
        )
    })?;

    tx.charge_gas_for_signature(serialized_tx.len(), meter)
        .map_err(|e| match e {
            TransactionVerificationError::GasError(_) => {
                AuthenticationError::OutOfGas(e.to_string())
            }
            _ => AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(e.to_string()),
                raw_tx_hash,
            ),
        })?;

    #[cfg(feature = "native")]
    if let Some(known_result) = SIGNATURE_CACHE.get(&(raw_tx_hash, *chain_hash)) {
        return known_result;
    }

    let res = tx
        .verify_signature_unmetered(&serialized_tx)
        .map_err(|e| match e {
            TransactionVerificationError::GasError(_) => {
                AuthenticationError::OutOfGas(e.to_string())
            }
            _ => AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(e.to_string()),
                raw_tx_hash,
            ),
        });

    #[cfg(feature = "native")]
    SIGNATURE_CACHE.insert((raw_tx_hash, *chain_hash), res.clone());

    res
}

/// Authenticate and verify deserialized sov-tx. See `authenticate`.
///
/// # Errors
/// Returns an error if gas runs out at any point, if deserialization or hashing fails, or if the
/// signature cannot be verified.
pub fn verify_and_decode_tx<S: Spec, D: DispatchCall<Spec = S>>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    chain_hash: &[u8; 32],
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    let resolved_hashes = crate::runtime::ResolvedChainHashes {
        primary: *chain_hash,
        grace_period_hashes: Vec::new(),
    };
    verify_and_decode_tx_multi_hash(raw_tx_hash, tx, resolved_hashes, meter)
}

/// Authenticate raw sov-transaction.
///
/// This function resolves the appropriate chain hashes for the current block height
/// using configured overrides (including grace periods), falling back to `default_chain_hash`
/// when no override applies. It tries verification with each valid hash until one succeeds.
///
/// # Errors
/// Returns an error if gas runs out at any point, if deserialization or hashing fails, or if the
/// signature cannot be verified with any of the valid chain hashes.
pub fn authenticate<
    Accessor: ProvableStateReader<User, Spec = S> + VersionReader,
    S: Spec,
    D: DispatchCall<Spec = S>,
>(
    mut raw_tx: &[u8],
    default_chain_hash: &[u8; 32],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    // Resolve all valid chain hashes for this height (including grace period hashes)
    let height = state.rollup_height_to_access();
    let resolved_hashes = resolve_chain_hashes_for_height(height.get(), *default_chain_hash);

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

    verify_and_decode_tx_multi_hash::<S, D>(raw_tx_hash, tx, resolved_hashes, state)
}

/// Authenticate and verify deserialized sov-tx with multiple chain hashes.
///
/// Tries verification with the primary hash first, then any grace period hashes.
/// Returns success on the first hash that verifies successfully.
fn verify_and_decode_tx_multi_hash<S: Spec, D: DispatchCall<Spec = S>>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    resolved_hashes: crate::runtime::ResolvedChainHashes,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    // Extract auth_data and verify chain_id (these don't depend on chain_hash)
    let (auth_data, details, runtime_call) = match &tx {
        Transaction::V0(tx_v0) => {
            let auth_data = tx_v0.auth_data(raw_tx_hash, meter)?;
            (auth_data, &tx_v0.details, &tx_v0.runtime_call)
        }
        Transaction::V1(tx_v1) => {
            let auth_data = tx_v1.auth_data(raw_tx_hash, meter)?;
            (auth_data, &tx_v1.details, &tx_v1.runtime_call)
        }
    };

    verify_chain_id(details, raw_tx_hash)?;

    // Try signature verification with each valid chain hash
    let mut last_error = None;
    for chain_hash in resolved_hashes.iter() {
        match verify_signature(&tx, chain_hash, raw_tx_hash, meter) {
            Ok(()) => {
                let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
                    raw_tx_hash,
                    authenticated_tx: details.clone().into(),
                };
                return Ok((tx_and_raw_hash, auth_data, runtime_call.clone()));
            }
            Err(AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(error),
                hash,
            )) => {
                last_error = Some(AuthenticationError::FatalError(
                    FatalError::SigVerificationFailed(error),
                    hash,
                ));
            }
            Err(e) => return Err(e),
        }
    }

    // All chain hashes failed - return the last error
    Err(last_error.expect("resolved_hashes always has at least one hash (the primary)"))
}

/// Authenticate raw unregistered sov-transaction.
pub fn authenticate_unregistered<
    Accessor: ProvableStateReader<sov_state::User, Spec = S> + VersionReader,
    S: Spec,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
>(
    raw_tx: &[u8],
    pre_exec_ws: &mut Accessor,
) -> Result<AuthenticationOutput<S, Rt::Decodable>, UnregisteredAuthenticationError> {
    let (tx_and_raw_hash, auth_data, runtime_call) =
        authenticate::<_, S, Rt>(raw_tx, &Rt::CHAIN_HASH, pre_exec_ws).map_err(|e| match e {
            AuthenticationError::FatalError(err, hash) => {
                UnregisteredAuthenticationError::FatalError(err, hash)
            }
            AuthenticationError::OutOfGas(err) => UnregisteredAuthenticationError::OutOfGas(err),
        })?;
    Ok((tx_and_raw_hash, auth_data, runtime_call))
}

/// Decode bytes as a Sovereign SDK transaction, returning the message and tx info.
#[cfg(feature = "native")]
pub fn decode_sov_tx<S: Spec, D: DispatchCall<Spec = S>>(
    raw_tx: &[u8],
) -> Result<D::Decodable, FatalError> {
    decode_sov_tx_with_cryptospec::<S, D, <S as Spec>::CryptoSpec>(raw_tx)
}
/// Decode bytes as a Sovereign SDK transaction, returning the message and tx info.
/// Allows decoding `Transaction`s with non-default `CryptoSpec` generics.
#[cfg(feature = "native")]
pub fn decode_sov_tx_with_cryptospec<S: Spec, D: DispatchCall<Spec = S>, C: CryptoSpecExt>(
    mut raw_tx: &[u8],
) -> Result<D::Decodable, FatalError> {
    let tx =
        <Transaction<D, S, C> as MeteredBorshDeserialize<S>>::unmetered_deserialize(&mut raw_tx)
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    Ok(tx.into_runtime_call())
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
