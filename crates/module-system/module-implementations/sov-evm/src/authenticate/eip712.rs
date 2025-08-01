use std::marker::PhantomData;

use sov_address::EvmCryptoSpec;
use sov_modules_api::capabilities::{
    self, BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator,
};
#[cfg(feature = "native")]
use sov_modules_api::FullyBakedTx;
use sov_modules_api::{DispatchCall, ProvableStateReader, RawTx, Runtime, Spec};
use sov_state::User;

/// EIP-712-compatible transaction authenticator. See [`TransactionAuthenticator`].
pub struct Eip712Authenticator<S, Rt>(PhantomData<(S, Rt)>);

impl<S, Rt> TransactionAuthenticator<S> for Eip712Authenticator<S, Rt>
where
    S: Spec<CryptoSpec = EvmCryptoSpec>,
    Rt: Runtime<S> + DispatchCall<Spec = S>,
{
    type Decodable = Rt::Decodable;
    type Input = RawTx;

    #[cfg(feature = "native")]
    fn decode_serialized_tx(
        tx: &FullyBakedTx,
    ) -> Result<Self::Decodable, sov_modules_api::capabilities::FatalError> {
        let tx: RawTx = borsh::from_slice(&tx.data)
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

        capabilities::decode_sov_tx::<S, Rt>(&tx.data)
    }

    fn authenticate<Accessor: ProvableStateReader<User, Spec = S>>(
        _tx: &FullyBakedTx,
        _state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        unimplemented!("authenticate");
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
        _batch: &BatchFromUnregisteredSequencer,
        _state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::UnregisteredAuthenticationError,
    > {
        unimplemented!("authenticate_unregistered");
    }

    fn add_standard_auth(tx: RawTx) -> Self::Input {
        tx
    }
}
