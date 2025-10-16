//! Transaction types that use Secp256k1CryptoSpec for signature and public key fields.
//!
//! This module provides transaction types that mirror the standard transaction structures
//! but use Secp256k1CryptoSpec types instead of the primary CryptoSpec. This allows
//! deserialization and verification of secp256k1-signed transactions even when the rollup's
//! primary CryptoSpec uses a different signature scheme (e.g., ed25519).
//!
//! Because this type exists primarily for correct deserialization, this means that
//! a) rollups that natively support secp256k1 don't strictly need this type, as the native
//!    `Transaction<D, S>` will be byte-identical to the `Secp256k1Transaction<D, S>`.
//! b) rollups that don't use a secp256k1-compatible Spec will have existing tooling that uses
//!    `Transaction<D, S>` remain incompatible with the EIP-712 authenticator (e.g. sov-cli).

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::transaction::{TransactionCallable, TxDetails, UnsignedTransaction};
use sov_modules_api::{
    CredentialId, GasMeter, GasSpec, MeteredBorshDeserialize, MeteredBorshDeserializeError,
    PublicKey, Spec,
};

use crate::crypto_markers::Secp256k1CryptoSpec;

/// A transaction structure that uses Secp256k1CryptoSpec types for signature and public key.
///
/// This mirrors the standard `Transaction` type but allows deserialization with secp256k1 crypto
/// when the rollup's primary CryptoSpec uses a different signature scheme.
///
/// *Important*: This must be manually kept in sync with `sov_modules_api::transaction::VersionedTx`.
#[derive(derive_more::Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize)]
pub enum Secp256k1Transaction<D: TransactionCallable, S: Spec>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    V0(Secp256k1Version0<D::Call, S>),
}

/// Version 0 transaction with secp256k1 signature and public key.
///
/// This mirrors `sov_modules_api::transaction::Version0` but uses Secp256k1CryptoSpec types.
#[derive(derive_more::Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize)]
pub struct Secp256k1Version0<Call, S: Spec>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    /// The secp256k1 signature of the transaction.
    pub signature: <S::CryptoSpec as Secp256k1CryptoSpec>::Signature,

    /// The secp256k1 public key of the sender.
    pub pub_key: <S::CryptoSpec as Secp256k1CryptoSpec>::PublicKey,

    /// The runtime call of the transaction.
    pub runtime_call: Call,

    /// Uniqueness identifier of this transaction. see [`UniquenessData`] for more details.
    pub uniqueness: UniquenessData,

    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
}

impl<D: TransactionCallable, S: Spec> Secp256k1Transaction<D, S>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    /// Verify the secp256k1 signature against the provided message.
    pub fn verify_signature(
        &self,
        msg: &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<(), sov_modules_api::transaction::TransactionVerificationError<S::Gas>> {
        match &self {
            Secp256k1Transaction::V0(inner) => {
                // Use MeteredSignature for gas metering
                sov_modules_api::MeteredSignature::new::<S>(inner.signature.clone())
                    .verify(&inner.pub_key, msg, meter)
                    .map_err(sov_modules_api::transaction::TransactionVerificationError::from)?;
            }
        }
        Ok(())
    }

    /// Convert to an unsigned transaction for EIP-712 hashing.
    pub fn to_unsigned_transaction(&self) -> UnsignedTransaction<D, S> {
        match &self {
            Secp256k1Transaction::V0(inner) => UnsignedTransaction {
                runtime_call: inner.runtime_call.clone(),
                details: inner.details.clone(),
                uniqueness: inner.uniqueness,
            },
        }
    }
}

impl<R: TransactionCallable, S: Spec> Secp256k1Transaction<R, S>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    fn unmetered_deserialize_inner(buf: &mut &[u8]) -> Result<Self, std::io::Error> {
        let this = <Secp256k1Transaction<R, S> as borsh::BorshDeserialize>::deserialize(buf)?;
        tracing::trace!(transaction = ?this, "Deserialized transaction");
        Ok(this)
    }
}

impl<R: TransactionCallable, S: Spec> MeteredBorshDeserialize<S> for Secp256k1Transaction<R, S>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    fn bias_borsh_deserialization() -> <S as Spec>::Gas {
        S::tx_bias_borsh_deserialization()
    }

    fn gas_to_charge_per_byte_borsh_deserialization() -> <S as Spec>::Gas {
        S::tx_gas_to_charge_per_byte_borsh_deserialization()
    }

    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        Self::charge_gas_to_deserialize(buf, meter)?;

        Secp256k1Transaction::<R, S>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    #[cfg(feature = "native")]
    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        Secp256k1Transaction::<R, S>::unmetered_deserialize_inner(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }
}

/// Derive a credential ID from the secp256k1 public key with gas metering.
///
/// This is analogous to `sov_modules_api::gas::metered_utils::metered_credential` but
/// uses the Secp256k1CryptoSpec public key type instead of the primary CryptoSpec.
pub fn metered_secp256k1_credential<S: Spec>(
    pub_key: &<S::CryptoSpec as Secp256k1CryptoSpec>::PublicKey,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<CredentialId, sov_modules_api::gas::GasMeteringError<S::Gas>>
where
    S::CryptoSpec: Secp256k1CryptoSpec,
{
    let cost = <S as sov_modules_api::GasSpec>::gas_to_charge_for_credential();
    meter.charge_gas(cost)?;
    Ok(pub_key.credential_id())
}

#[cfg(test)]
mod tests {
    use sov_modules_api::transaction::{Transaction, TransactionCallable};

    use super::*;
    /// This test ensures Secp256k1Transaction stays in sync with VersionedTx.
    ///
    /// If this fails to compile, new versions have been added to VersionedTx
    /// and must be added to Secp256k1Transaction as well.
    #[test]
    fn eip712_tx_is_kept_in_sync_assertion() {
        // This will fail to compile if VersionedTx gets new variants
        fn _reminder_to_update_definition_when_new_transaction_variants_are_added<
            D: TransactionCallable,
            S: Spec,
        >(
            tx: Transaction<D, S>,
        ) {
            match tx {
                Transaction::V0(_) => {
                    // If you're adding a new version here (e.g., V1), also add it to Secp256k1Transaction!
                } // Compiler will error here if new versions are added
            }
        }
    }
}
