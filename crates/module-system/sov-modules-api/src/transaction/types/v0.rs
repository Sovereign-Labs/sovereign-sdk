use crate::transaction::data::TxDetails;
use derivative::Derivative;
use sov_universal_wallet::UniversalWallet;

use crate::capabilities::{AuthenticationError, AuthorizationData, UniquenessData};
use crate::transaction::{
    hex_field_format, Credentials, Transaction, TransactionCallable, UnsignedTransaction,
    UnsignedTransactionV0,
};
use crate::{metered_credential, CryptoSpecExt, GasMeter, Spec, TxHash};

#[derive(
    derive_more::Debug,
    Clone,
    Derivative,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[derivative(
    PartialEq(bound = "R::Call: PartialEq + Eq"),
    Eq(bound = "R::Call: PartialEq + Eq")
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
/// V0 transaction.
pub struct Version0<R: TransactionCallable, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// The signature of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub signature: C::Signature,
    /// The public key of the sender of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub pub_key: C::PublicKey,
    /// The runtime call of the transaction.
    #[sov_wallet(bound = "R::Call: sov_universal_wallet::schema::UniversalWallet")]
    pub runtime_call: R::Call,
    /// Uniqueness identifier of this transaction. see [`UniquenessData`] for more details.
    pub uniqueness: UniquenessData,
    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
    /// Signer-declared address override.
    /// See [`crate::capabilities::AuthorizationData::address_override`] for routing semantics.
    #[serde(default)]
    pub address_override: Option<S::Address>,
}

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Version0<R, S, C> {
    /// Extracts the versioned unsigned transaction data from this signed envelope.
    pub fn as_unsigned(&self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction::V0(UnsignedTransactionV0 {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            address_override: self.address_override,
        })
    }

    /// Extracts authorization data from this transaction.
    pub fn auth_data<M: GasMeter<Spec = S>>(
        &self,
        raw_tx_hash: TxHash,
        non_malleable_hash: TxHash,
        meter: &mut M,
    ) -> Result<AuthorizationData<S>, AuthenticationError> {
        let pub_key = self.pub_key.clone();
        let credential_id = metered_credential::<S, C>(&pub_key, meter)
            .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

        Ok(AuthorizationData {
            uniqueness: self.uniqueness,
            tx_hash: raw_tx_hash,
            non_malleable_hash,
            credential_id,
            credentials: Credentials::new(pub_key),
            default_address: credential_id.into(),
            address_override: self.address_override,
        })
    }
}

impl<R: TransactionCallable, S: Spec> From<Version0<R, S>> for Transaction<R, S> {
    fn from(value: Version0<R, S>) -> Self {
        Transaction::V0(value)
    }
}
