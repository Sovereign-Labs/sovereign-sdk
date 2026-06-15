//! A `TransactionAuthenticator` implementation that delegates directly to the methods of
//! `EvmAuthenticator` and `SolanaOffchainAuthenticator`.
//! Since our authenticators are not currently easily composable, the authenticator implementations
//! provide convenience TransactionAuthenticator impls that can dispatch to either the standard
//! authenticator or the bespoke implementation. Since here we want to wire up *three* variants
//! (standard, EVM and SolanaOffchain), we need to re-write our own boilerplate combining the
//! three.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::capabilities::{
    self, BatchFromUnregisteredSequencer, TransactionAuthenticator, UnregisteredAuthenticationError,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::Base58Address;
use sov_modules_api::{
    DispatchCall, FullyBakedTx, GetGasPrice, ProvableStateReader, RawTx, Runtime, Spec,
    VersionReader,
};
use sov_state::User;
use std::marker::PhantomData;

/// See [`TransactionAuthenticator::Input`].
#[derive(std::fmt::Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum EvmAndSolanaOffchainAuthenticatorInput<T = RawTx, U = RawTx> {
    /// Authenticate using the `EVM` authenticator, which expects a standard EVM transaction
    /// (i.e. an rlp-encoded payload signed using secp256k1 and hashed using keccak256).
    Evm(T),
    /// Authenticate using the solana offchain authenticator, which expects a standard solana
    /// offchain message version 0 (ASCII, max 1212 bytes); we expect the ASCII message to contain
    /// a JSON-serialized transaction
    SolanaOffchain(U),
    /// Authenticate using the standard `sov-module` authenticator, which uses the default
    /// signature scheme and hashing algorithm defined in the rollup's [`Spec`].
    Standard(U),
}

/// Solana offchain message and EVM compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct EvmAndSolanaOffchainAuthenticator<S, Rt>(PhantomData<(S, Rt)>);

impl<S, Rt> TransactionAuthenticator<S> for EvmAndSolanaOffchainAuthenticator<S, Rt>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
    <Rt as DispatchCall>::Decodable: Serialize + DeserializeOwned,
{
    type Decodable = EvmAndSolanaOffchainAuthenticatorInput<
        sov_evm::CallMessage<S>,
        <Rt as DispatchCall>::Decodable,
    >;
    type Input = EvmAndSolanaOffchainAuthenticatorInput;

    fn authenticate<Accessor>(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    >
    where
        Accessor: ProvableStateReader<User, Spec = S>
            + GetGasPrice<Spec = S>
            + sov_modules_api::VersionReader,
    {
        let input: EvmAndSolanaOffchainAuthenticatorInput =
            borsh::from_slice(&tx.data).map_err(|e| {
                sov_modules_api::capabilities::fatal_deserialization_error::<_, S, _>(
                    &tx.data, e, state,
                )
            })?;

        match input {
            EvmAndSolanaOffchainAuthenticatorInput::Evm(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_evm::authenticate::<_, _>(&tx.data, state)?;

                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAndSolanaOffchainAuthenticatorInput::Evm(runtime_call),
                ))
            }
            EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_solana_offchain_auth::authentication::authenticate::<_, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        config_value!("CHAIN_NAME"),
                        state,
                    )?;

                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(runtime_call),
                ))
            }
            EvmAndSolanaOffchainAuthenticatorInput::Standard(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_modules_api::capabilities::authenticate::<_, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        state,
                    )?;

                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAndSolanaOffchainAuthenticatorInput::Standard(runtime_call),
                ))
            }
        }
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(
        tx: &sov_modules_api::FullyBakedTx,
    ) -> anyhow::Result<sov_modules_api::TxHash> {
        let input: EvmAndSolanaOffchainAuthenticatorInput = borsh::from_slice(&tx.data)?;

        match input {
            EvmAndSolanaOffchainAuthenticatorInput::Evm(tx) => {
                let (_rlp, tx) = sov_evm::decode_evm_tx(&tx.data)?;
                Ok(sov_rollup_interface::TxHash::new(**tx.hash()))
            }
            EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(tx)
            | EvmAndSolanaOffchainAuthenticatorInput::Standard(tx) => {
                Ok(capabilities::calculate_hash::<S>(&tx.data))
            }
        }
    }

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        let auth_variant: EvmAndSolanaOffchainAuthenticatorInput = borsh::from_slice(&tx.data)
            .map_err(|e| {
                sov_modules_api::capabilities::FatalError::DeserializationFailed(e.to_string())
            })?;

        match &auth_variant {
            EvmAndSolanaOffchainAuthenticatorInput::Evm(raw_tx) => {
                let (call, _tx) = sov_evm::decode_evm_tx(&raw_tx.data)?;
                Ok(EvmAndSolanaOffchainAuthenticatorInput::Evm(
                    sov_evm::CallMessage::<S>::Call(call),
                ))
            }
            EvmAndSolanaOffchainAuthenticatorInput::Standard(raw_tx) => {
                let call = capabilities::decode_sov_tx::<S, Rt>(&raw_tx.data)?;
                Ok(EvmAndSolanaOffchainAuthenticatorInput::Standard(call))
            }
            EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(raw_tx) => {
                let call = sov_solana_offchain_auth::authentication::decode_solana_json_tx::<S, Rt>(
                    &raw_tx.data,
                )?;
                Ok(EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(call))
            }
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
            Self::Input::SolanaOffchain(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_solana_offchain_auth::authentication::authenticate::<Accessor, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        config_value!("CHAIN_NAME"),
                        state,
                    )?;
                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAndSolanaOffchainAuthenticatorInput::SolanaOffchain(runtime_call),
                ))
            }
            Self::Input::Evm(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_evm::authenticate::<_, _>(&tx.data, state)?;
                Ok((
                    tx_and_raw_hash,
                    auth_data,
                    EvmAndSolanaOffchainAuthenticatorInput::Evm(runtime_call),
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
                    EvmAndSolanaOffchainAuthenticatorInput::Standard(runtime_call),
                ))
            }
        }
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        EvmAndSolanaOffchainAuthenticatorInput::Standard(tx)
    }
}
