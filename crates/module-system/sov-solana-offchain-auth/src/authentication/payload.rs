use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::transaction::{
    PriorityFeeBips, TransactionCallable, TxDetails, UnsignedTransaction,
};
use sov_modules_api::{Amount, SafeString, Spec};

/// The V0 Solana-specific payload that wallets sign as JSON.
///
/// This duplicates the fields from [`UnsignedTransaction`] instead of wrapping it, so that the JSON
/// displayed to users stays flat. The extra `chain_name` field is signed as part of the payload,
/// ensuring the destination chain is visible to the user before signing.
#[serde_with::serde_as]
#[derive(Debug, Serialize, Deserialize, UniversalWallet)]
#[serde(
    deny_unknown_fields,
    bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned"
)]
pub struct SolanaOffchainSigningPayloadV0<R: TransactionCallable, S: Spec> {
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
    /// Signer-declared address override.
    /// See [`sov_modules_api::capabilities::AuthorizationData::address_override`] for routing semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_override: Option<S::Address>,
    /// Message format version. Must be `0` for this struct.
    #[serde(deserialize_with = "deserialize_version_0")]
    pub version: u8,
}

impl<R, S> SolanaOffchainSigningPayloadV0<R, S>
where
    S: Spec,
    R: TransactionCallable,
    <R as TransactionCallable>::Call: Serialize + DeserializeOwned,
{
    pub(super) fn into_unsigned_transaction(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
            address_override: self.address_override,
        }
    }

    pub(super) fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<SolanaOffchainSigningPayloadV0<R, S>>(buf)
    }
}

/// The V1 Solana-specific payload that multisig wallets sign as JSON.
///
/// V1 extends the V0 payload with a signed multisig credential and explicit message format version.
#[serde_with::serde_as]
#[derive(Debug, Serialize, Deserialize, UniversalWallet)]
#[serde(
    deny_unknown_fields,
    bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned"
)]
pub struct SolanaOffchainSigningPayloadV1<R: TransactionCallable, S: Spec> {
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
    /// Signer-declared address override.
    /// See [`sov_modules_api::capabilities::AuthorizationData::address_override`] for routing semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_override: Option<S::Address>,
    /// Message format version. Must be `1` for this struct.
    #[serde(deserialize_with = "deserialize_version_1")]
    pub version: u8,
}

fn deserialize_version_0<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let v = <u8 as serde::Deserialize>::deserialize(deserializer)?;
    if v != 0 {
        return Err(serde::de::Error::custom(format!(
            "expected message version 0, got {v}"
        )));
    }
    Ok(v)
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

impl<R, S> SolanaOffchainSigningPayloadV1<R, S>
where
    S: Spec,
    R: TransactionCallable,
    <R as TransactionCallable>::Call: Serialize + DeserializeOwned,
{
    pub(super) fn into_unsigned_transaction(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
            address_override: self.address_override,
        }
    }

    pub(super) fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<SolanaOffchainSigningPayloadV1<R, S>>(buf)
    }
}

/// The pre-fork (state version 0) single-signer payload.
///
/// It differs from [`SolanaOffchainSigningPayloadV0`] in exactly the ways #2892 changed the V0
/// format: `details` carries the rollup `chain_id` instead of a `chain_hash_fragment`, and there is
/// no `version` or `address_override` field. The authenticator accepts it below
/// `ACCEPT_LEGACY_V0_TXS_UNTIL_HEIGHT` so that wallets can lag behind the hard fork; see
/// [`super::authenticate`]. `deny_unknown_fields` ensures a malformed current-format payload is
/// never reinterpreted as a legacy one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, bound = "R::Call: serde::de::DeserializeOwned")]
pub struct LegacySolanaOffchainSigningPayloadV0<R: TransactionCallable, S: Spec> {
    /// The runtime call
    pub runtime_call: R::Call,
    /// The uniqueness identifier
    pub uniqueness: UniquenessData,
    /// Pre-fork fee and gas details.
    pub details: LegacyTxDetails<S>,
    /// The chain name; see [`SolanaOffchainSigningPayloadV0::chain_name`].
    pub chain_name: SafeString,
}

/// The pre-fork [`TxDetails`], which committed to the rollup's `CHAIN_ID` rather than to a fragment
/// of the chain hash.
#[derive(Debug, Deserialize)]
#[serde(bound = "S: Spec")]
pub struct LegacyTxDetails<S: Spec> {
    /// See [`TxDetails::max_priority_fee_bips`].
    pub max_priority_fee_bips: PriorityFeeBips,
    /// See [`TxDetails::max_fee`].
    pub max_fee: Amount,
    /// See [`TxDetails::gas_limit`].
    pub gas_limit: Option<S::Gas>,
    /// The rollup chain id the signer committed to.
    pub chain_id: u64,
}

impl<R, S> LegacySolanaOffchainSigningPayloadV0<R, S>
where
    S: Spec,
    R: TransactionCallable,
    <R as TransactionCallable>::Call: DeserializeOwned,
{
    /// Converts into the current transaction shape. Like the standard authenticator's legacy
    /// decoding, the signed `chain_id` is carried in `chain_hash_fragment` so it can be checked
    /// against `CHAIN_ID`; a legacy payload never carries an address override.
    pub(super) fn into_unsigned_transaction(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: TxDetails {
                max_priority_fee_bips: self.details.max_priority_fee_bips,
                max_fee: self.details.max_fee,
                gas_limit: self.details.gas_limit,
                chain_hash_fragment: self.details.chain_id,
            },
            address_override: None,
        }
    }

    pub(super) fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<LegacySolanaOffchainSigningPayloadV0<R, S>>(buf)
    }
}
