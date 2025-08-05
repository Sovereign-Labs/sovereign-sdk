use std::marker::PhantomData;

use alloy_primitives::address;
use alloy_sol_types::{eip712_domain, Eip712Domain, SolStruct};
use sov_address::EvmCryptoSpec;
use sov_modules_api::capabilities::{
    self, calculate_hash_metered, extract_authorization_data, verify_chain_id, AuthenticationError,
    AuthenticationOutput, BatchFromUnregisteredSequencer, FatalError, TransactionAuthenticator,
    UnregisteredAuthenticationError,
};
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, Transaction, TransactionVerificationError, VersionedTx,
};
use sov_modules_api::{
    DispatchCall, FullyBakedTx, GasMeter, MeteredBorshDeserialize, MeteredBorshDeserializeError,
    ProvableStateReader, RawTx, Runtime, Spec, TxHash,
};
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
        tx: &FullyBakedTx,
        state: &mut Accessor,
    ) -> Result<
        capabilities::AuthenticationOutput<S, Self::Decodable>,
        capabilities::AuthenticationError,
    > {
        let tx: RawTx = borsh::from_slice(&tx.data).map_err(|e| {
            capabilities::fatal_deserialization_error::<_, S, _>(&tx.data, e, state)
        })?;

        authenticate::<_, S, Rt>(&tx.data, state)
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

        Ok(authenticate::<_, S, Rt>(&tx.data, state)?)
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
>(
    mut raw_tx: &[u8],
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
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
    verify_and_decode_tx::<S, D>(raw_tx_hash, tx, state)
}

fn verify_and_decode_tx<S: Spec<CryptoSpec = EvmCryptoSpec>, D: DispatchCall<Spec = S>>(
    raw_tx_hash: TxHash,
    tx: Transaction<D, S>,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
    match &tx.versioned_tx {
        VersionedTx::V0(tx_v0) => {
            verify_chain_id(tx_v0, raw_tx_hash)?;
            verify_eip712_signature(&tx, raw_tx_hash, meter)?;
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

fn verify_eip712_signature<S: Spec<CryptoSpec = EvmCryptoSpec>, D: DispatchCall<Spec = S>>(
    tx: &Transaction<D, S>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
    let unsigned = tx.to_unsigned_transaction();
    let tx_details = unsigned.details.as_sol_struct();

    pub const DOMAIN: Eip712Domain = eip712_domain! {
        name: "Transaction",
        version: "1",
        chain_id: 4321,
        verifying_contract: address!("0000000000000000000000000000000000000000"),
    };
    let eip712_hash = tx_details.eip712_signing_hash(&DOMAIN);

    tx.verify_signature(eip712_hash.as_slice(), meter)
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
