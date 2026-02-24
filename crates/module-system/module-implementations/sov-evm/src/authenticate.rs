use std::marker::PhantomData;

use crate::conversions::RlpConversionError;
use crate::Evm;
use crate::TransactionSigned;
use crate::{call, CallMessage, RlpEvmTransaction};
use alloy_consensus::{transaction::SignerRecoverable, Transaction};
use alloy_primitives::Address;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::capabilities::{
    self, fatal_deserialization_error, AuthenticationOutput, AuthorizationData,
    BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator, UniquenessData,
    UnregisteredAuthenticationError,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::runtime::capabilities::AuthenticationError;
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, Credentials, PriorityFeeBips, TxDetails,
};
use sov_modules_api::StateReader;
use sov_modules_api::VersionReader;
use sov_modules_api::{
    DispatchCall, FullyBakedTx, Gas, GetGasPrice, ProvableStateReader, RawTx, Runtime, Spec,
};
use sov_rollup_interface::TxHash;
use sov_state::User;

#[cfg(feature = "native")]
use sov_modules_api::capabilities::{SignatureVerificationCache, DEFAULT_SIGNATURE_CACHE_SIZE};

/// At default size, and ~70 bytes per entry, this costs about 17.5 MB at the default size.
#[cfg(feature = "native")]
static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<Address>> =
    std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));

const UNSUPPORTED_TRANSACTION_TYPE_MESSAGE: &str =
    "Unsupported transaction type: only EIP-1559 and EIP-7702 are supported.";

impl<S: Spec> Evm<S> {
    /// Validates the user's max fee per gas against the rollup's base fee and returns a gas multiplier.
    ///
    /// The max fee check is only enforced when:
    /// - The current block number exceeds `EVM_MAX_FEE_CHECK_HEIGHT`
    /// - The check has not been disabled via an admin `UpdateRuntimeConfig` message
    ///
    /// Returns:
    /// - `Ok(1)` if the fee check passes (user fee >= rollup base fee)
    /// - `Ok(100)` if the fee check is skipped (before height threshold or disabled)
    /// - `Err(InsufficientMaxFeePerGas)` if the user's fee is below the rollup base fee
    fn validate_fee_and_calculate_multiplier<Accessor: StateReader<User> + VersionReader>(
        &self,
        user_max_fee_per_gas: u128,
        rollup_base_fee: u128,
        tx_hash: TxHash,
        state: &mut Accessor,
    ) -> Result<u64, AuthenticationError> {
        let apply_max_fee_check_after_height: u64 = config_value!("EVM_MAX_FEE_CHECK_HEIGHT");

        let block_number: u64 = state.rollup_height_to_access().get();

        if block_number > apply_max_fee_check_after_height {
            let is_max_fee_check_disabled = self.is_max_fee_check_disabled(state).map_err(|e| {
                AuthenticationError::OutOfGas(format!("validate_fee_and_calculate_multiplier: {e}"))
            })?;
            if is_max_fee_check_disabled {
                return Ok(100);
            }
            if user_max_fee_per_gas < rollup_base_fee {
                let err = FatalError::InsufficientMaxFeePerGas {
                    user_max_fee_per_gas,
                    rollup_base_fee,
                };
                return Err(AuthenticationError::FatalError(err, tx_hash));
            }

            Ok(1)
        } else {
            Ok(100)
        }
    }
}

/// Recovers the signer from an EVM transaction.
fn recover_evm_signer(
    tx: &TransactionSigned,
    tx_hash: TxHash,
) -> Result<Address, AuthenticationError> {
    #[cfg(feature = "native")]
    if let Some(known_result) = SIGNATURE_CACHE.get(&tx_hash) {
        return known_result;
    }

    let result = tx.recover_signer().map_err(|_| {
        AuthenticationError::FatalError(
            FatalError::SigVerificationFailed(format!(
                "Invalid ethereum signature: tx hash {tx_hash}"
            )),
            tx_hash,
        )
    });
    #[cfg(feature = "native")]
    SIGNATURE_CACHE.insert(tx_hash, result.clone());

    result
}

pub(crate) fn ensure_supported_transaction_type(tx: &TransactionSigned) -> Result<(), FatalError> {
    if tx.is_eip1559() || tx.is_eip7702() {
        return Ok(());
    }

    Err(FatalError::DeserializationFailed(
        UNSUPPORTED_TRANSACTION_TYPE_MESSAGE.to_string(),
    ))
}

/// Creates the transaction details and tx hash for an EVM transaction.
fn create_auth_tx_and_hash<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    tx: &TransactionSigned,
    gas_price: <<S as Spec>::Gas as Gas>::Price,
    state: &mut Accessor,
) -> Result<AuthenticatedTransactionAndRawHash<S>, AuthenticationError> {
    let tx_hash = TxHash::new(**tx.hash());
    ensure_supported_transaction_type(tx)
        .map_err(|err| AuthenticationError::FatalError(err, tx_hash))?;
    let evm = Evm::<S>::default();
    let tx_chain_id = validate_chain_id(tx.chain_id(), tx.is_eip7702(), tx_hash)?;

    let user_max_fee_per_gas = tx.max_fee_per_gas();
    let rollup_base_fee = gas_price.as_ref()[0].0;

    let multiplier = evm.validate_fee_and_calculate_multiplier(
        user_max_fee_per_gas,
        rollup_base_fee,
        tx_hash,
        state,
    )?;

    let gas_limit = tx.gas_limit().saturating_mul(multiplier);
    let gas_limit: <S as Spec>::Gas = [gas_limit, gas_limit].into();

    let max_fee = gas_limit
        .checked_value(gas_price)
        .ok_or(AuthenticationError::FatalError(
            FatalError::Other("Amount overflow".into()),
            tx_hash,
        ))?;

    let tx_details = TxDetails {
        chain_id: tx_chain_id,
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        max_fee,
        gas_limit: Some(gas_limit),
    };

    Ok(AuthenticatedTransactionAndRawHash {
        raw_tx_hash: tx_hash,
        authenticated_tx: tx_details.into(),
    })
}

fn validate_chain_id(
    tx_chain_id: Option<u64>,
    allow_zero_chain_id: bool,
    tx_hash: TxHash,
) -> Result<u64, AuthenticationError> {
    let rollup_chain_id = config_value!("CHAIN_ID");
    let tx_chain_id = tx_chain_id.ok_or(AuthenticationError::FatalError(
        FatalError::MissingChainId(rollup_chain_id),
        tx_hash,
    ))?;

    // EIP-7702 permits chain-id 0 signatures; keep that exception narrow.
    if tx_chain_id != rollup_chain_id && !(allow_zero_chain_id && tx_chain_id == 0) {
        return Err(AuthenticationError::FatalError(
            FatalError::InvalidChainId {
                expected: rollup_chain_id,
                got: tx_chain_id,
            },
            tx_hash,
        ));
    }

    Ok(tx_chain_id)
}

/// Extracts EVM authorization data from a verified transaction.
fn extract_evm_authorization_data<S: Spec>(
    signer: Address,
    tx_hash: TxHash,
    nonce: u64,
) -> AuthorizationData<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let ethereum_address: EthereumAddress = signer.into();
    let credentials = Credentials::new(signer);
    let credential_id = ethereum_address.as_credential_id();

    AuthorizationData {
        uniqueness: UniquenessData::Nonce(nonce),
        tx_hash,
        credential_id,
        credentials,
        default_address: S::Address::from_vm_address(ethereum_address),
    }
}

/// Authenticates a raw evm transaction.
/// # Security
///
/// If the caller does plan to derive rollup addresses from evm addresses, they should be sure that their scheme for doing so is deterministic and
/// collision resistant. You don't want someone to be able to pick a rollup address that someone else is already using!
pub fn authenticate<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    raw_tx: &[u8],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, CallMessage<S>>, AuthenticationError>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    // TODO: Charge gas for deserialization & signature check.

    let (rlp, tx) = decode_evm_tx(raw_tx)
        .map_err(|e| fatal_deserialization_error::<Accessor, S, _>(raw_tx, e, state))?;

    let gas_price = state.gas_price();
    let tx_and_raw_hash = create_auth_tx_and_hash(&tx, gas_price, state)?;

    let signer = recover_evm_signer(&tx, tx_and_raw_hash.raw_tx_hash)?;

    let nonce = tx.nonce();
    let auth_data = extract_evm_authorization_data::<S>(signer, tx_and_raw_hash.raw_tx_hash, nonce);

    let call = CallMessage::<S>::Call(rlp);

    tracing::debug!(
        nonce,
        credential_id = ?auth_data.credential_id,
        raw_tx_hash = ?tx_and_raw_hash.raw_tx_hash,
        "Received and authenticated EVM transaction"
    );

    Ok((tx_and_raw_hash, auth_data, call))
}

/// Decode a byte sequence into an EVM transaction without checking the signature
pub fn decode_evm_tx(raw_tx: &[u8]) -> Result<(RlpEvmTransaction, TransactionSigned), FatalError> {
    let tx_data = RlpEvmTransaction::try_from_slice(raw_tx)
        .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    if tx_data.rlp.is_empty() {
        return Err(FatalError::DeserializationFailed(
            RlpConversionError::EmptyRawTx.to_string(),
        ));
    }

    let tx = crate::convert_to_tx_signed(tx_data.clone())
        .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;
    ensure_supported_transaction_type(&tx)?;

    Ok((tx_data, tx))
}

/// Indicates that a runtime supports the `Ethereum` transaction authenticator
/// and provides suitable methods for encoding and decoding Ethereum transactions.
pub trait EthereumAuthenticator<S: Spec>: Runtime<S> {
    /// Add the Ethereum discriminant to a transaction the runtime.
    fn add_ethereum_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input;

    /// Encode a transaction with the Ethereum discriminant for the runtime.
    fn encode_with_ethereum_auth(tx: RawTx) -> FullyBakedTx {
        <Self::Auth as TransactionAuthenticator<S>>::encode_authenticator_input(
            &Self::add_ethereum_auth(tx),
        )
    }
}

/// See [`TransactionAuthenticator::Input`].
#[derive(std::fmt::Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum EvmAuthenticatorInput<T = RawTx, U = RawTx> {
    /// Authenticate using the `EVM` authenticator, which expects a standard EVM transaction
    /// (i.e. an rlp-encoded payload signed using secp256k1 and hashed using keccak256).
    Evm(T),
    /// Authenticate using the standard `sov-module` authenticator, which uses the default
    /// signature scheme and hashing algorithm defined in the rollup's [`Spec`].
    Standard(U),
}

/// EVM-compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct EvmAuthenticator<S, Rt>(PhantomData<(S, Rt)>);

impl<S, Rt> TransactionAuthenticator<S> for EvmAuthenticator<S, Rt>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
{
    type Decodable = EvmAuthenticatorInput<call::CallMessage<S>, <Rt as DispatchCall>::Decodable>;
    type Input = EvmAuthenticatorInput;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        let auth_variant: EvmAuthenticatorInput = borsh::from_slice(&tx.data).map_err(|e| {
            sov_modules_api::capabilities::FatalError::DeserializationFailed(e.to_string())
        })?;

        match auth_variant {
            EvmAuthenticatorInput::Evm(raw_tx) => {
                let (call, _tx) = decode_evm_tx(&raw_tx.data)?;
                Ok(EvmAuthenticatorInput::Evm(call::CallMessage::<S>::Call(
                    call,
                )))
            }
            EvmAuthenticatorInput::Standard(raw_tx) => {
                let call = capabilities::decode_sov_tx::<S, Rt>(&raw_tx.data)?;
                Ok(EvmAuthenticatorInput::Standard(call))
            }
        }
    }

    fn authenticate<
        Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    >(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        let input: EvmAuthenticatorInput = borsh::from_slice(&tx.data).map_err(|e| {
            sov_modules_api::capabilities::fatal_deserialization_error::<_, S, _>(
                &tx.data, e, state,
            )
        })?;

        match input {
            EvmAuthenticatorInput::Evm(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    authenticate::<_, _>(&tx.data, state)?;

                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAuthenticatorInput::Evm(runtime_call),
                ))
            }
            EvmAuthenticatorInput::Standard(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_modules_api::capabilities::authenticate::<_, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        state,
                    )?;

                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAuthenticatorInput::Standard(runtime_call),
                ))
            }
        }
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(
        tx: &sov_modules_api::FullyBakedTx,
    ) -> anyhow::Result<sov_modules_api::TxHash> {
        let input: EvmAuthenticatorInput = borsh::from_slice(&tx.data)?;

        match input {
            EvmAuthenticatorInput::Evm(tx) => {
                let (_rlp, tx) = decode_evm_tx(&tx.data)?;
                Ok(TxHash::new(**tx.hash()))
            }
            EvmAuthenticatorInput::Standard(tx) => Ok(capabilities::calculate_hash::<S>(&tx.data)),
        }
    }

    fn authenticate_unregistered<
        Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
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
            Self::Input::Evm(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    authenticate::<_, _>(&tx.data, state)?;
                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAuthenticatorInput::Evm(runtime_call),
                ))
            }
            Self::Input::Standard(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_modules_api::capabilities::authenticate_unregistered::<_, S, Rt>(
                        &tx.data, state,
                    )?;
                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAuthenticatorInput::Standard(runtime_call),
                ))
            }
        }
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        EvmAuthenticatorInput::Standard(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_evm_tx, ensure_supported_transaction_type, validate_chain_id, RlpEvmTransaction,
    };
    use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559, TxEip2930, TxEip7702, TxLegacy};
    use alloy_eips::Encodable2718;
    use alloy_primitives::Signature;
    use sov_modules_api::macros::config_value;
    use sov_modules_api::runtime::capabilities::{AuthenticationError, FatalError};
    use sov_rollup_interface::TxHash;

    fn encoded(tx: EthereumTxEnvelope<alloy_consensus::TxEip4844>) -> Vec<u8> {
        borsh::to_vec(&RlpEvmTransaction {
            rlp: tx.encoded_2718().to_vec(),
        })
        .unwrap()
    }

    #[test]
    fn validate_chain_id_rejects_missing_chain_id() {
        let tx_hash = TxHash::new([0u8; 32]);
        let err = validate_chain_id(None, false, tx_hash).unwrap_err();
        let expected = config_value!("CHAIN_ID");
        assert_eq!(
            err,
            AuthenticationError::FatalError(FatalError::MissingChainId(expected), tx_hash)
        );
    }

    #[test]
    fn validate_chain_id_allows_zero_only_when_explicitly_enabled() {
        let tx_hash = TxHash::new([1u8; 32]);
        assert!(validate_chain_id(Some(0), false, tx_hash).is_err());
        assert_eq!(validate_chain_id(Some(0), true, tx_hash).unwrap(), 0);
    }

    #[test]
    fn decode_evm_tx_accepts_eip1559() {
        let raw = encoded(EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559::default(),
            Signature::test_signature(),
            Default::default(),
        )));
        assert!(decode_evm_tx(&raw).is_ok());
    }

    #[test]
    fn decode_evm_tx_accepts_eip7702() {
        let raw = encoded(EthereumTxEnvelope::Eip7702(Signed::new_unchecked(
            TxEip7702::default(),
            Signature::test_signature(),
            Default::default(),
        )));
        assert!(decode_evm_tx(&raw).is_ok());
    }

    #[test]
    fn decode_evm_tx_rejects_legacy() {
        let raw = encoded(EthereumTxEnvelope::Legacy(Signed::new_unchecked(
            TxLegacy::default(),
            Signature::test_signature(),
            Default::default(),
        )));
        let err = decode_evm_tx(&raw).unwrap_err();
        assert!(matches!(err, FatalError::DeserializationFailed(_)));
    }

    #[test]
    fn decode_evm_tx_rejects_eip2930() {
        let raw = encoded(EthereumTxEnvelope::Eip2930(Signed::new_unchecked(
            TxEip2930::default(),
            Signature::test_signature(),
            Default::default(),
        )));
        let err = decode_evm_tx(&raw).unwrap_err();
        assert!(matches!(err, FatalError::DeserializationFailed(_)));
    }

    #[test]
    fn decode_evm_tx_rejects_eip4844() {
        let raw = encoded(EthereumTxEnvelope::Eip4844(Signed::new_unchecked(
            alloy_consensus::TxEip4844::default(),
            Signature::test_signature(),
            Default::default(),
        )));
        let err = decode_evm_tx(&raw).unwrap_err();
        assert!(matches!(err, FatalError::DeserializationFailed(_)));
    }

    #[test]
    fn ensure_supported_transaction_type_rejects_eip4844() {
        let tx = EthereumTxEnvelope::Eip4844(Signed::new_unchecked(
            alloy_consensus::TxEip4844::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let err = ensure_supported_transaction_type(&tx).unwrap_err();
        assert!(matches!(err, FatalError::DeserializationFailed(_)));
    }
}
