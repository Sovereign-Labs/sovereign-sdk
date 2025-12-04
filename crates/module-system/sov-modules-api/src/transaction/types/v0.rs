use crate::transaction::data::TxDetails;
use derivative::Derivative;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::capabilities::{AuthenticationError, AuthorizationData, UniquenessData};
use crate::transaction::{hex_field_format, Credentials, Transaction, TransactionCallable};
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
    PartialEq(bound = "Call: PartialEq + Eq"),
    Eq(bound = "Call: PartialEq + Eq")
)]
#[serde(bound = "Call: serde::Serialize + serde::de::DeserializeOwned")]
/// V0 transaction.
pub struct Version0<Call, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// The signature of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub signature: C::Signature,
    /// The public key of the sender of the transaction.
    #[serde(with = "hex_field_format")]
    #[sov_wallet(display = "hex")]
    pub pub_key: C::PublicKey,
    /// The runtime call of the transaction.
    #[sov_wallet(
        bound = "Call: sov_rollup_interface::sov_universal_wallet::schema::UniversalWallet"
    )]
    pub runtime_call: Call,
    /// Uniqueness identifier of this transaction. see [`UniquenessData`] for more details.
    pub uniqueness: UniquenessData,
    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
}

impl<Call, S: Spec, C: CryptoSpecExt> Version0<Call, S, C> {
    /// Extracts authorization data from this transaction.
    pub fn auth_data<M: GasMeter<Spec = S>>(
        &self,
        raw_tx_hash: TxHash,
        meter: &mut M,
    ) -> Result<AuthorizationData<S>, AuthenticationError> {
        let pub_key = self.pub_key.clone();
        let credential_id = metered_credential::<S, C>(&pub_key, meter)
            .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

        Ok(AuthorizationData {
            uniqueness: self.uniqueness,
            tx_hash: raw_tx_hash,
            credential_id,
            credentials: Credentials::new(pub_key),
            default_address: credential_id.into(),
        })
    }
}

impl<R: TransactionCallable, S: Spec> From<Version0<R::Call, S>> for Transaction<R, S> {
    fn from(value: Version0<R::Call, S>) -> Self {
        Transaction::V0(value)
    }
}
