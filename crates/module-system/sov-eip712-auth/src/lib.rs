use std::marker::PhantomData;
use std::sync::OnceLock;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::capabilities::{
    self, calculate_hash_metered, verify_chain_id, AuthenticationError, AuthenticationOutput,
    BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator,
    UnregisteredAuthenticationError,
};
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, Transaction, TransactionVerificationError,
};
use sov_modules_api::{
    DispatchCall, FullyBakedTx, GasMeter, MeteredBorshDeserialize, MeteredBorshDeserializeError,
    ProvableStateReader, RawTx, Runtime, Spec, TxHash, VersionReader,
};
use sov_state::User;

mod crypto_markers;
pub use crate::crypto_markers::{CryptoSpecWithSecp256k1, Secp256k1CryptoSpec};

#[cfg(feature = "native")]
mod stub_evm_rpc;
#[cfg(feature = "native")]
pub use stub_evm_rpc::stub_evm_rpc;

#[cfg(feature = "native")]
use sov_modules_api::capabilities::{SignatureVerificationCache, DEFAULT_SIGNATURE_CACHE_SIZE};

#[cfg(feature = "native")]
static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<()>> =
    std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));

/// The length of an EIP712 signing hash in bytes.
/// EIP712 hashes are 66 bytes: 1 byte prefix (0x19) + 1 byte version (0x01) + 32 bytes domain separator + 32 bytes struct hash.
const EIP712_HASH_LENGTH: usize = 66;

/// Trait for providing schema to the EIP-712 authenticator.
pub trait SchemaProvider {
    /// The schema as borsh-serialized bytes, typically from build-time generation.
    const SCHEMA_BORSH: &'static [u8];

    /// Get the parsed schema, initializing it lazily from the borsh bytes.
    ///
    /// This uses a function-local static to ensure the schema is parsed at most
    /// once per program execution (or once per zkVM block reset).
    fn get_schema() -> &'static Schema {
        static SCHEMA: OnceLock<Schema> = OnceLock::new();

        SCHEMA.get_or_init(|| {
            borsh::from_slice(Self::SCHEMA_BORSH)
                .expect("Failed to parse serialized schema data (SCHEMA_BORSH)")
        })
    }
}

/// Indicates that a runtime supports the Eip712 transaction authenticator
/// and provides suitable methods for encoding and decoding EIP712-signed transactions.
pub trait Eip712AuthenticatorTrait<S: Spec>: Runtime<S> {
    /// Add the Solana offchain discriminant to a transaction the runtime.
    fn add_eip712_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input;

    /// Encode a transaction with the Solana offchain discriminant for the runtime.
    fn encode_with_eip712_auth(tx: RawTx) -> FullyBakedTx {
        <Self::Auth as TransactionAuthenticator<S>>::encode_authenticator_input(
            &Self::add_eip712_auth(tx),
        )
    }
}

/// EIP-712-compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct Eip712Authenticator<S, D, SP>(PhantomData<(S, D, SP)>);

/// See [`TransactionAuthenticator::Input`].
#[derive(std::fmt::Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum Eip712AuthenticatorInput {
    /// Authenticate using EIP712 an signature.
    Eip712(RawTx),
    /// Authenticate using the standard `sov-module` authenticator.
    Standard(RawTx),
}

impl<S, Rt, SP> TransactionAuthenticator<S> for Eip712Authenticator<S, Rt, SP>
where
    S: Spec<CryptoSpec: Secp256k1CryptoSpec>,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
    SP: SchemaProvider,
{
    type Decodable = Rt::Decodable;
    type Input = Eip712AuthenticatorInput;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        let auth_variant: Eip712AuthenticatorInput = borsh::from_slice(&tx.data).map_err(|e| {
            sov_modules_api::capabilities::FatalError::DeserializationFailed(e.to_string())
        })?;

        match auth_variant {
            Eip712AuthenticatorInput::Standard(raw_tx) => {
                sov_modules_api::capabilities::decode_sov_tx::<S, Rt>(&raw_tx.data)
            }
            Eip712AuthenticatorInput::Eip712(raw_tx) => {
                sov_modules_api::capabilities::decode_sov_tx_with_cryptospec::<
                    S,
                    Rt,
                    <<S as Spec>::CryptoSpec as Secp256k1CryptoSpec>::CryptoSpec,
                >(&raw_tx.data)
            }
        }
    }

    fn authenticate<Accessor: ProvableStateReader<User, Spec = S> + VersionReader>(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        let input: Eip712AuthenticatorInput = borsh::from_slice(&tx.data).map_err(|e| {
            sov_modules_api::capabilities::fatal_deserialization_error::<_, S, _>(
                &tx.data, e, state,
            )
        })?;

        match input {
            Eip712AuthenticatorInput::Eip712(tx) => authenticate::<_, S, Rt, SP>(&tx.data, state),
            Eip712AuthenticatorInput::Standard(tx) => {
                sov_modules_api::capabilities::authenticate::<_, S, Rt>(
                    &tx.data,
                    &Rt::CHAIN_HASH,
                    state,
                )
            }
        }
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(
        tx: &sov_modules_api::FullyBakedTx,
    ) -> anyhow::Result<sov_modules_api::TxHash> {
        let input: Eip712AuthenticatorInput = borsh::from_slice(&tx.data)?;

        // Here we intentionally use S::CryptoSpec for hashing, as transaction hashes don't need to
        // use the Secp256k1CryptoSpec
        match input {
            Eip712AuthenticatorInput::Eip712(tx) | Eip712AuthenticatorInput::Standard(tx) => {
                Ok(sov_modules_api::capabilities::calculate_hash::<S>(&tx.data))
            }
        }
    }

    fn authenticate_unregistered<
        Accessor: ProvableStateReader<User, Spec = S> + sov_modules_api::GetGasPrice<Spec = S>,
    >(
        batch: &BatchFromUnregisteredSequencer,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::UnregisteredAuthenticationError,
    > {
        match borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?
        {
            Self::Input::Eip712(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    authenticate::<_, S, Rt, SP>(&tx.data, state)?;
                Ok((tx_and_raw_hash, auth_data, runtime_call))
            }
            Self::Input::Standard(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_modules_api::capabilities::authenticate_unregistered::<_, S, Rt>(
                        &tx.data, state,
                    )?;
                Ok((tx_and_raw_hash, auth_data, runtime_call))
            }
        }
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        Eip712AuthenticatorInput::Standard(tx)
    }
}

/// Authenticate raw sov-transaction signed as EIP712 typed data.
///
/// # Errors
/// Returns an error if gas runs out at any point, if deserialization or hashing fails, or if the
/// signature cannot be verified.
pub fn authenticate<
    Accessor: ProvableStateReader<User, Spec = S>,
    S: Spec<CryptoSpec: Secp256k1CryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    raw_tx: &[u8],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    let raw_tx_hash = calculate_hash_metered::<Accessor, S>(raw_tx, state)
        .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    let tx = match <Transaction<D, S, <S::CryptoSpec as Secp256k1CryptoSpec>::CryptoSpec> as MeteredBorshDeserialize<S>>::deserialize(
        &mut &raw_tx[..],
        state,
    ) {
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
    verify_and_decode_tx::<S, D, SP>(raw_tx_hash, tx, state)
}

fn verify_and_decode_tx<
    S: Spec<CryptoSpec: Secp256k1CryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S, <S::CryptoSpec as Secp256k1CryptoSpec>::CryptoSpec>,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
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
    verify_eip712_signature::<S, D, SP>(&tx, raw_tx_hash, meter)?;

    let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
        raw_tx_hash,
        authenticated_tx: details.clone().into(),
    };

    Ok((tx_and_raw_hash, auth_data, runtime_call.clone()))
}

fn get_eip712_hash<
    S: Spec<CryptoSpec: Secp256k1CryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    tx: &Transaction<D, S, <S::CryptoSpec as Secp256k1CryptoSpec>::CryptoSpec>,
    raw_tx_hash: TxHash,
) -> Result<[u8; EIP712_HASH_LENGTH], AuthenticationError> {
    // Convert the transaction to unsigned transaction (removes signature)
    let unsigned_tx = tx.to_unsigned_transaction();

    // Serialize the unsigned transaction - this is what should be signed
    let unsigned_tx_bytes = borsh::to_vec(&unsigned_tx).map_err(|e| {
        AuthenticationError::FatalError(
            FatalError::SigVerificationFailed(format!(
                "Failed to serialize unsigned transaction: {e}"
            )),
            raw_tx_hash,
        )
    })?;

    // Use the schema provider to get the schema and calculate the EIP712 signing hash
    let schema = SP::get_schema();
    let transaction_type_index = schema.rollup_expected_index(sov_modules_api::sov_universal_wallet::schema::RollupRoots::UnsignedTransaction)
         .map_err(|e| AuthenticationError::FatalError(
             FatalError::SigVerificationFailed(format!("Cannot verify EIP712 signature. Failed to get UnsignedTransaction type from schema: {e}")),
             raw_tx_hash,
         ))?;

    let eip712_hash = schema
        .eip712_signing_digest(transaction_type_index, &unsigned_tx_bytes)
        .map_err(|e| {
            AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(format!("Failed to calculate EIP712 hash: {e}")),
                raw_tx_hash,
            )
        })?;

    Ok(eip712_hash)
}

fn verify_eip712_signature<
    S: Spec<CryptoSpec: Secp256k1CryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    tx: &Transaction<D, S, <S::CryptoSpec as Secp256k1CryptoSpec>::CryptoSpec>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
    tx.charge_gas_for_signature(EIP712_HASH_LENGTH, meter)
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
    if let Some(known_result) = SIGNATURE_CACHE.get(&raw_tx_hash) {
        return known_result;
    }

    let eip712_hash = get_eip712_hash::<S, D, SP>(tx, raw_tx_hash)?;
    let res = tx
        .verify_signature_unmetered(&eip712_hash)
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
    SIGNATURE_CACHE.insert(raw_tx_hash, res.clone());

    res
}
