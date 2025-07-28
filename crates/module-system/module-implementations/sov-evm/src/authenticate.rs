use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use reth_primitives::TransactionSigned;
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
use sov_modules_api::{
    Amount, DispatchCall, FullyBakedTx, ProvableStateReader, RawTx, Runtime, Spec,
};
use sov_rollup_interface::TxHash;
use sov_state::User;

use crate::conversions::RlpConversionError;
use crate::{call, CallMessage, RlpEvmTransaction};

/// Authenticates a raw evm transaction.
/// # Security
///
/// If the caller does plan to derive rollup addresses from evm addresses, they should be sure that their scheme for doing so is deterministic and
/// collision resistant. You don't want someone to be able to pick a rollup address that someone else is already using!
pub fn authenticate<Accessor: ProvableStateReader<User, Spec = S>, S: Spec>(
    raw_tx: &[u8],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, CallMessage>, AuthenticationError>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    // TODO: Charge gas for deserialization & signature check.

    let (rlp, tx) = decode_evm_tx(raw_tx)
        .map_err(|e| fatal_deserialization_error::<Accessor, S, _>(raw_tx, e, state))?;
    let hash = TxHash::new(tx.hash().into());
    let signer = tx.recover_signer().ok_or(AuthenticationError::FatalError(
        FatalError::SigVerificationFailed(format!("Invalid ethereum signature: tx hash {hash}")),
        hash,
    ))?;

    let chain_id = config_value!("CHAIN_ID");
    let max_priority_fee_bips = PriorityFeeBips::ZERO;
    let max_fee = Amount::new(10_000_000);
    let gas_limit = None;

    let nonce = tx.nonce();

    let credentials = Credentials::new(signer);
    let credential_id = signer.into_word().0.into();

    let authenticated_tx = AuthenticatedTransactionData::<S>(TxDetails {
        chain_id,
        max_priority_fee_bips,
        max_fee,
        gas_limit,
    });

    let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
        raw_tx_hash: hash,
        authenticated_tx,
    };

    let ethereum_address: EthereumAddress = signer.into();
    let auth_data = AuthorizationData {
        uniqueness: UniquenessData::Nonce(nonce),
        tx_hash: hash,
        credential_id,
        credentials,
        default_address: S::Address::from_vm_address(ethereum_address),
    };
    let call = CallMessage { rlp };
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

    let tx = TransactionSigned::decode_enveloped(&mut &tx_data.rlp[..])
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
    type Decodable = EvmAuthenticatorInput<call::CallMessage, <Rt as DispatchCall>::Decodable>;
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
                Ok(EvmAuthenticatorInput::Evm(call::CallMessage { rlp: call }))
            }
            EvmAuthenticatorInput::Standard(raw_tx) => {
                let call = capabilities::decode_sov_tx::<S, Rt>(&raw_tx.data)?;
                Ok(EvmAuthenticatorInput::Standard(call))
            }
        }
    }

    fn authenticate<Accessor: ProvableStateReader<User, Spec = S>>(
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
                Ok(TxHash::new(tx.hash().into()))
            }
            EvmAuthenticatorInput::Standard(tx) => Ok(capabilities::calculate_hash(
                &tx.data,
                &mut sov_modules_api::gas::UnlimitedGasMeter::<S>::default(),
            )?),
        }
    }

    fn authenticate_unregistered<Accessor: ProvableStateReader<User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::UnregisteredAuthenticationError,
    > {
        let Self::Input::Standard(input) = borsh::from_slice(&batch.tx.data)
            .map_err(|_| UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant)?
        else {
            return Err(UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant);
        };

        let (tx_and_raw_hash, auth_data, runtime_call) =
            sov_modules_api::capabilities::authenticate::<_, S, Rt>(
                &input.data,
                &Rt::CHAIN_HASH,
                state,
            )
            .map_err(|e| match e {
                AuthenticationError::FatalError(err, hash) => {
                    UnregisteredAuthenticationError::FatalError(err, hash)
                }
                AuthenticationError::OutOfGas(err) => {
                    UnregisteredAuthenticationError::OutOfGas(err)
                }
            })?;

        if Rt::allow_unregistered_tx(&runtime_call) {
            Ok((
                tx_and_raw_hash,
                auth_data,
                EvmAuthenticatorInput::Standard(runtime_call),
            ))
        } else {
            Err(UnregisteredAuthenticationError::FatalError(
                FatalError::Other(
                    "The runtime call included in the transaction was invalid.".to_string(),
                ),
                tx_and_raw_hash.raw_tx_hash,
            ))?
        }
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        EvmAuthenticatorInput::Standard(tx)
    }
}
