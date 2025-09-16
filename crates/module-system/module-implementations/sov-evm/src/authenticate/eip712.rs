use std::marker::PhantomData;
use std::sync::OnceLock;

use sov_address::EvmCryptoSpec;
use sov_modules_api::capabilities::{
    self, calculate_hash_metered, extract_authorization_data, verify_chain_id, AuthenticationError,
    AuthenticationOutput, BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator,
    UnregisteredAuthenticationError,
};
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, Transaction, TransactionVerificationError, VersionedTx,
};
use sov_modules_api::{
    DispatchCall, FullyBakedTx, GasMeter, MeteredBorshDeserialize, MeteredBorshDeserializeError,
    ProvableStateReader, RawTx, Spec, TxHash,
};
use sov_state::User;

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

/// EIP-712-compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct Eip712Authenticator<S, D, SP>(PhantomData<(S, D, SP)>);

impl<S, D, SP> TransactionAuthenticator<S> for Eip712Authenticator<S, D, SP>
where
    S: Spec<CryptoSpec = EvmCryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
{
    type Decodable = D::Decodable;
    type Input = RawTx;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        let tx: RawTx = borsh::from_slice(&tx.data)
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

        capabilities::decode_sov_tx::<S, D>(&tx.data)
    }

    fn authenticate<Accessor: ProvableStateReader<User, Spec = S>>(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        let tx: RawTx = borsh::from_slice(&tx.data).map_err(|e| {
            capabilities::fatal_deserialization_error::<_, S, _>(&tx.data, e, state)
        })?;

        authenticate::<_, S, D, SP>(&tx.data, state)
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(
        tx: &sov_modules_api::FullyBakedTx,
    ) -> anyhow::Result<sov_modules_api::TxHash> {
        use sov_modules_api::capabilities::calculate_hash;

        let tx: RawTx = borsh::from_slice(&tx.data)?;
        Ok(calculate_hash::<S>(&tx.data))
    }

    fn authenticate_unregistered<Accessor: ProvableStateReader<User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::UnregisteredAuthenticationError,
    > {
        let tx: RawTx = borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?;

        Ok(authenticate::<_, S, D, SP>(&tx.data, state)?)
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        tx
    }
}

/// Authenticate raw sov-transaction signed as EIP712 typed data.
///
/// # Errors
/// Returns an error if gas runs out at any point, if deserialization or hashing fails, or if the
/// signature cannot be verified.
pub fn authenticate<
    Accessor: ProvableStateReader<User, Spec = S>,
    S: Spec<CryptoSpec = EvmCryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    raw_tx: &[u8],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    let raw_tx_hash = calculate_hash_metered::<Accessor, S>(raw_tx, state)
        .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    let tx = match <Transaction<D, S> as MeteredBorshDeserialize<S>>::deserialize(
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
    S: Spec<CryptoSpec = EvmCryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    match &tx.versioned_tx {
        VersionedTx::V0(tx_v0) => {
            verify_chain_id(&tx_v0.details, raw_tx_hash)?;
            verify_eip712_signature::<S, D, SP>(&tx, raw_tx_hash, meter)?;
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

fn verify_eip712_signature<
    S: Spec<CryptoSpec = EvmCryptoSpec>,
    D: DispatchCall<Spec = S>,
    SP: SchemaProvider,
>(
    tx: &Transaction<D, S>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
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
        .eip712_signing_hash(transaction_type_index, &unsigned_tx_bytes)
        .map_err(|e| {
            AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(format!("Failed to calculate EIP712 hash: {e}")),
                raw_tx_hash,
            )
        })?;

    tx.verify_signature(&eip712_hash, meter)
        .map_err(|e| match e {
            TransactionVerificationError::GasError(_) => {
                AuthenticationError::OutOfGas(e.to_string())
            }
            _ => AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(e.to_string()),
                raw_tx_hash,
            ),
        })
}
