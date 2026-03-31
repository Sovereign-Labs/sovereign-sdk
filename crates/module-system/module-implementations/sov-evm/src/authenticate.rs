use std::marker::PhantomData;

use crate::conversions::RlpConversionError;
use crate::Evm;
use crate::TransactionSigned;
use crate::{call, CallMessage, RlpEvmTransaction};
use alloy_consensus::{transaction::SignerRecoverable, Transaction};
use alloy_eips::eip2718::{Decodable2718, EIP1559_TX_TYPE_ID};
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
    AuthenticatedTransactionAndRawHash, AuthenticatedTransactionData, Credentials, PriorityFeeBips,
    TxDetails,
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
        use crate::sov_fee_and_gas_utils::GasMultiplier;

        let multiplier = self.fee_multiplier(state).map_err(|e| {
            AuthenticationError::OutOfGas(format!("validate_fee_and_calculate_multiplier: {e}"))
        })?;

        match multiplier {
            GasMultiplier::FeeCheckActive if user_max_fee_per_gas < rollup_base_fee => {
                let err = FatalError::InsufficientMaxFeePerGas {
                    user_max_fee_per_gas,
                    rollup_base_fee,
                };
                Err(AuthenticationError::FatalError(err, tx_hash))
            }
            _ => Ok(multiplier.as_u64()),
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

/// Builds authenticated transaction details for an EVM transaction.
fn build_authenticated_tx_data<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    tx_hash: TxHash,
    tx_chain_id: u64,
    gas_limit: u64,
    user_max_fee_per_gas: u128,
    state: &mut Accessor,
) -> Result<AuthenticatedTransactionData<S>, AuthenticationError> {
    let gas_price = state.gas_price();
    let rollup_base_fee = gas_price.as_ref()[0].0;

    let evm = Evm::<S>::default();
    let multiplier = evm.validate_fee_and_calculate_multiplier(
        user_max_fee_per_gas,
        rollup_base_fee,
        tx_hash,
        state,
    )?;

    let gas_limit = gas_limit
        .checked_mul(multiplier)
        .ok_or(AuthenticationError::FatalError(
            FatalError::Other("Gas limit overflow".into()),
            tx_hash,
        ))?;
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

    Ok(tx_details.into())
}

/// Creates the transaction details and tx hash for an EVM transaction.
fn create_auth_tx_and_hash<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    tx: &TransactionSigned,
    state: &mut Accessor,
) -> Result<AuthenticatedTransactionAndRawHash<S>, AuthenticationError> {
    let tx_hash = TxHash::new(**tx.hash());
    let tx_chain_id = validate_chain_id(tx.chain_id(), tx_hash)?;
    if let Some(max_priority_fee) = tx.max_priority_fee_per_gas() {
        if max_priority_fee > tx.max_fee_per_gas() {
            return Err(AuthenticationError::FatalError(
                FatalError::Other("max priority fee per gas higher than max fee per gas".into()),
                tx_hash,
            ));
        }
    }

    let authenticated_tx = build_authenticated_tx_data::<_, S>(
        tx_hash,
        tx_chain_id,
        tx.gas_limit(),
        tx.max_fee_per_gas(),
        state,
    )?;

    Ok(AuthenticatedTransactionAndRawHash {
        raw_tx_hash: tx_hash,
        authenticated_tx,
    })
}

fn validate_chain_id(
    tx_chain_id: Option<u64>,
    tx_hash: TxHash,
) -> Result<u64, AuthenticationError> {
    let rollup_chain_id = config_value!("CHAIN_ID");
    let tx_chain_id = tx_chain_id.ok_or(AuthenticationError::FatalError(
        FatalError::MissingChainId(rollup_chain_id),
        tx_hash,
    ))?;

    if tx_chain_id != rollup_chain_id {
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

#[cfg(feature = "native")]
fn resolve_request_preflight_nonce<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    request: &alloy_rpc_types::TransactionRequest,
    from: Address,
    tx_hash: TxHash,
    state: &mut Accessor,
) -> Result<u64, AuthenticationError> {
    if let Some(nonce) = request.nonce {
        return Ok(nonce);
    }

    let credential_id = EthereumAddress::from(from).as_credential_id();
    sov_uniqueness::Uniqueness::<S>::default()
        .next_nonce(&credential_id, state)
        .map_err(|e| {
            AuthenticationError::FatalError(
                FatalError::Other(format!("resolve_request_preflight_nonce: {e}")),
                tx_hash,
            )
        })
}

/// Builds authenticated transaction data and authorization data for an RPC
/// request that already has an explicit sender, gas limit, and fee field.
#[cfg(feature = "native")]
pub fn build_request_preflight_auth<
    Accessor: ProvableStateReader<User, Spec = S> + GetGasPrice<Spec = S> + VersionReader,
    S: Spec,
>(
    request: &alloy_rpc_types::TransactionRequest,
    state: &mut Accessor,
) -> Result<(AuthenticatedTransactionData<S>, AuthorizationData<S>), AuthenticationError>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    // Sentinel hash for error reporting only — no real signed transaction exists
    // in the RPC preflight path.
    let sentinel_tx_hash = TxHash::new([0; 32]);
    let from = request.from.ok_or(AuthenticationError::FatalError(
        FatalError::Other("Missing from address".into()),
        sentinel_tx_hash,
    ))?;
    let gas_limit = request.gas.ok_or(AuthenticationError::FatalError(
        FatalError::Other("Missing gas limit".into()),
        sentinel_tx_hash,
    ))?;
    let user_max_fee_per_gas =
        request
            .max_fee_per_gas
            .or(request.gas_price)
            .ok_or(AuthenticationError::FatalError(
                FatalError::Other("Missing gas price".into()),
                sentinel_tx_hash,
            ))?;
    let tx_chain_id = match request.chain_id {
        Some(chain_id) => validate_chain_id(Some(chain_id), sentinel_tx_hash)?,
        None => config_value!("CHAIN_ID"),
    };
    let authenticated_tx = build_authenticated_tx_data::<_, S>(
        sentinel_tx_hash,
        tx_chain_id,
        gas_limit,
        user_max_fee_per_gas,
        state,
    )?;
    let nonce = resolve_request_preflight_nonce::<_, S>(request, from, sentinel_tx_hash, state)?;
    let auth_data = extract_evm_authorization_data::<S>(from, sentinel_tx_hash, nonce);

    Ok((authenticated_tx, auth_data))
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

    let tx_and_raw_hash = create_auth_tx_and_hash(&tx, state)?;

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

    let type_tag = TransactionSigned::extract_type_byte(&mut &tx_data.rlp[..]).unwrap_or(0); // Reject as a legacy transaction by default
    if type_tag != EIP1559_TX_TYPE_ID {
        return Err(FatalError::DeserializationFailed(
            "Invalid transaction type: Only EIP-1559 is currently supported. If you need to use EIP-7702, please reach out to the SDK developers for support.".to_string(),
        ));
    }
    let tx = TransactionSigned::decode_2718_exact(&tx_data.rlp)
        .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

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
