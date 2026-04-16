use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::transaction::{
    TransactionCallable, TxDetails, UnsignedTransaction, UnsignedTransactionV0,
    UnsignedTransactionV1,
};
use sov_modules_api::{SafeString, Spec};

/// The payload for a solana offchain message.
/// Essentially a wrapper around `sov_modules_api::transaction::UnsignedTransactionV0` that also
/// includes the chain_name, in order to ensure the name gets displayed to the user and signed as
/// part of the message.
/// We duplicate the UnsignedTransactionV0 type rather than wrapping it to ensure the JSON displayed
/// to the user doesn't get too nested.
#[serde_with::serde_as]
#[derive(Debug, Serialize, Deserialize, UniversalWallet)]
#[serde(
    deny_unknown_fields,
    bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned"
)]
pub struct SolanaOffchainUnsignedTransactionV0<R: TransactionCallable, S: Spec> {
    /// The runtime call
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
    /// The chain name, so that users can verify the destination chain and avoid replay attacks
    /// from malicious chains (if the chain name matches some other chain the use but didn't expect
    /// to be signing for right now).
    pub chain_name: SafeString,
}

impl<R, S> SolanaOffchainUnsignedTransactionV0<R, S>
where
    S: Spec,
    R: TransactionCallable,
    <R as TransactionCallable>::Call: Serialize + DeserializeOwned,
{
    pub(super) fn into_unsigned_tx(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction::V0(UnsignedTransactionV0 {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
        })
    }

    pub(super) fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<SolanaOffchainUnsignedTransactionV0<R, S>>(buf)
    }
}

#[serde_with::serde_as]
#[derive(Debug, Serialize, Deserialize, UniversalWallet)]
#[serde(
    deny_unknown_fields,
    bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned"
)]
pub struct SolanaOffchainUnsignedTransactionV1<R: TransactionCallable, S: Spec> {
    /// The runtime call
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
    /// The chain name, so that users can verify the destination chain and avoid replay attacks
    /// from malicious chains (if the chain name matches some other chain the use but didn't expect
    /// to be signing for right now).
    pub chain_name: SafeString,
    /// The multisig credential derived from the multisig parameters (hash of min_signers + sorted
    /// pubkeys), formatted in the rollup's native address format. Included in the signed message
    /// so that signers commit to the multisig configuration and prevent credential malleability
    /// from reusing signed bytes in a different multisig envelope.
    /// This is the "multisig address" except if the credential is mapped to another address in
    /// `sov-accounts`.
    pub multisig_id: S::Address,
    /// Message format version. Must be `1` for this struct.
    #[serde(deserialize_with = "deserialize_version_1")]
    pub version: u8,
}

fn deserialize_version_1<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let v = <u8 as serde::Deserialize>::deserialize(deserializer)?;
    if v != 1 {
        return Err(serde::de::Error::custom(format!(
            "expected message version 1, got {v}"
        )));
    }
    Ok(v)
}

impl<R, S> SolanaOffchainUnsignedTransactionV1<R, S>
where
    S: Spec,
    R: TransactionCallable,
    <R as TransactionCallable>::Call: Serialize + DeserializeOwned,
{
    pub(super) fn into_unsigned_tx(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction::V1(UnsignedTransactionV1 {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
            credential_address: self.multisig_id,
        })
    }

    pub(super) fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<SolanaOffchainUnsignedTransactionV1<R, S>>(buf)
    }
}
