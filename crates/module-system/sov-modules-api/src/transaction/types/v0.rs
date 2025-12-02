use crate::transaction::data::TxDetails;
use derivative::Derivative;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

use crate::capabilities::UniquenessData;
use crate::transaction::{hex_field_format, Transaction, TransactionCallable};
use crate::{CryptoSpecExt, Spec};

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

impl<R: TransactionCallable, S: Spec> From<Version0<R::Call, S>> for Transaction<R, S> {
    fn from(value: Version0<R::Call, S>) -> Self {
        Transaction::V0(value)
    }
}
