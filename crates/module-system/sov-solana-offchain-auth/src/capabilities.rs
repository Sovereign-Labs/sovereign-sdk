use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::capabilities::{
    AuthenticationError, BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator,
    UnregisteredAuthenticationError,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{DispatchCall, FullyBakedTx, ProvableStateReader, RawTx, Runtime, Spec};

/// Indicates that a runtime supports the `SolanaOffchain` transaction authenticator
/// and provides suitable methods for encoding and decoding solana offchain message transactions.
pub trait SolanaOffchainAuthenticatorTrait<S: Spec>: Runtime<S> {
    /// Add the Solana offchain discriminant to a transaction the runtime.
    fn add_solana_offchain_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input;

    /// Encode a transaction with the Solana offchain discriminant for the runtime.
    fn encode_with_solana_offchain_auth(tx: RawTx) -> FullyBakedTx {
        <Self::Auth as TransactionAuthenticator<S>>::encode_authenticator_input(
            &Self::add_solana_offchain_auth(tx),
        )
    }
}

/// See [`TransactionAuthenticator::Input`].
#[derive(std::fmt::Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum SolanaOffchainAuthenticatorInput<T = RawTx> {
    /// Authenticate using the standard `sov-module` authenticator, which uses the default
    /// signature scheme and hashing algorithm defined in the rollup's [`Spec`].
    Standard(T),
    /// Authenticate using the solana offchain authenticator, which expects a standard solana
    /// offchain message version 0 (ASCII, max 1212 bytes); we expect the ASCII message to contain
    /// a JSON-serialized transaction
    SolanaOffchain(T),
}

/// Solana offchain message compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct SolanaOffchainAuthenticator<S, Rt>(PhantomData<(S, Rt)>);

impl<S, Rt> TransactionAuthenticator<S> for SolanaOffchainAuthenticator<S, Rt>
where
    S: Spec,
    // S::Address: FromVmAddress<Base58Address>,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
    <Rt as DispatchCall>::Decodable: Serialize + DeserializeOwned,
{
    type Decodable = <Rt as DispatchCall>::Decodable;
    type Input = SolanaOffchainAuthenticatorInput;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        use crate::authentication::decode_solana_json_tx;

        let auth_variant: SolanaOffchainAuthenticatorInput =
            borsh::from_slice(&tx.data).map_err(|e| {
                sov_modules_api::capabilities::FatalError::DeserializationFailed(e.to_string())
            })?;

        match auth_variant {
            SolanaOffchainAuthenticatorInput::Standard(raw_tx) => {
                let call = sov_modules_api::capabilities::decode_sov_tx::<S, Rt>(&raw_tx.data)?;
                Ok(call)
            }
            SolanaOffchainAuthenticatorInput::SolanaOffchain(raw_tx) => {
                let call = decode_solana_json_tx::<S, Rt>(&raw_tx.data)?;
                Ok(call)
            }
        }
    }

    fn authenticate<Accessor: ProvableStateReader<sov_state::User, Spec = S>>(
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        sov_modules_api::capabilities::AuthenticationOutput<S, Self::Decodable>,
        sov_modules_api::capabilities::AuthenticationError,
    > {
        let input: SolanaOffchainAuthenticatorInput = borsh::from_slice(&tx.data).map_err(|e| {
            sov_modules_api::capabilities::fatal_deserialization_error::<_, S, _>(
                &tx.data, e, state,
            )
        })?;

        match input {
            SolanaOffchainAuthenticatorInput::SolanaOffchain(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    crate::authentication::authenticate::<Accessor, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        config_value!("CHAIN_NAME"),
                        state,
                    )?;

                Ok((tx_and_raw_hash, auth_data, runtime_call))
            }
            SolanaOffchainAuthenticatorInput::Standard(tx) => {
                let (tx_and_raw_hash, auth_data, runtime_call) =
                    sov_modules_api::capabilities::authenticate::<_, S, Rt>(
                        &tx.data,
                        &Rt::CHAIN_HASH,
                        state,
                    )?;

                Ok((tx_and_raw_hash, auth_data, runtime_call))
            }
        }
    }

    #[cfg(feature = "native")]
    fn compute_tx_hash(
        tx: &sov_modules_api::FullyBakedTx,
    ) -> anyhow::Result<sov_modules_api::TxHash> {
        let input: SolanaOffchainAuthenticatorInput = borsh::from_slice(&tx.data)?;

        match input {
            SolanaOffchainAuthenticatorInput::SolanaOffchain(tx)
            | SolanaOffchainAuthenticatorInput::Standard(tx) => {
                Ok(sov_modules_api::capabilities::calculate_hash::<S>(&tx.data))
            }
        }
    }

    fn authenticate_unregistered<Accessor: ProvableStateReader<sov_state::User, Spec = S>>(
        batch: &BatchFromUnregisteredSequencer,
        state: &mut Accessor,
    ) -> Result<
        sov_modules_api::capabilities::AuthenticationOutput<S, Self::Decodable>,
        UnregisteredAuthenticationError,
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
            Ok((tx_and_raw_hash, auth_data, runtime_call))
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
        SolanaOffchainAuthenticatorInput::Standard(tx)
    }
}
