use std::io;
use std::num::TryFromIntError;

use digest::consts::U32;
use digest::Digest;
use serde::de::DeserializeOwned;
use sov_rollup_interface::crypto::{CredentialId, SigVerificationError, Signature};
use sov_rollup_interface::zk::CryptoSpec;
use thiserror::Error;

use crate::gas::traits::{Gas, GasMeter, GasMeteringError};
use crate::{as_u32_or_panic, GasSpec, PublicKey, Spec};

/// A metered hasher that charges gas for each operation.
/// This data structure should be used in the module system to charge gas when hashing data.
pub struct MeteredHasher<'a, Meter: GasMeter, Hasher: Digest<OutputSize = U32>> {
    inner: Hasher,
    meter: &'a mut Meter,
    gas_to_charge_for_hash_update: <Meter::Spec as Spec>::Gas,
    gas_to_charge_per_byte_for_hash_update: <Meter::Spec as Spec>::Gas,
}

type GasUnit<S> = <S as Spec>::Gas;
type MeteringError<M> = GasMeteringError<GasUnit<<M as GasMeter>::Spec>>;

impl<'a, Meter: GasMeter, Hasher: Digest<OutputSize = U32>> MeteredHasher<'a, Meter, Hasher> {
    /// Create a new metered hasher from a given gas meter with default gas prices [`GasSpec::gas_to_charge_hash_update`] and [`GasSpec::gas_to_charge_per_byte_hash_update`]
    pub fn new(meter: &'a mut Meter) -> Self {
        Self::new_with_custom_price(
            meter,
            Meter::Spec::gas_to_charge_hash_update(),
            Meter::Spec::gas_to_charge_per_byte_hash_update(),
        )
    }

    /// Create a new metered hasher from a given gas meter with custom gas prices.
    pub fn new_with_custom_price(
        meter: &'a mut Meter,
        gas_to_charge_for_hash_update: <Meter::Spec as Spec>::Gas,
        gas_to_charge_per_byte_for_hash_update: <Meter::Spec as Spec>::Gas,
    ) -> Self {
        Self {
            inner: Hasher::new(),
            meter,
            gas_to_charge_for_hash_update,
            gas_to_charge_per_byte_for_hash_update,
        }
    }

    /// Update the [`MeteredHasher`] with the given data. Performs the same operation as [`Digest::update`] but charges gas.
    ///
    /// # Errors
    /// Returns an error if charging gas for the update operation fails.
    pub fn update(&mut self, data: &[u8]) -> Result<(), MeteringError<Meter>> {
        self.meter.charge_gas(&self.gas_to_charge_for_hash_update)?;
        self.meter.charge_linear_gas(
            &self.gas_to_charge_per_byte_for_hash_update,
            data.len()
                .try_into()
                .map_err(|e: TryFromIntError| MeteringError::<Meter>::Overflow(e.to_string()))?,
        )?;
        self.inner.update(data);
        Ok(())
    }

    /// Finalize the [`MeteredHasher`] and return the hash. Performs the same operation as [`Digest::finalize`] but charges gas.
    ///
    /// # Errors
    /// Returns an error if charging gas for the hashing operation fails.
    pub fn finalize(self) -> Result<[u8; 32], (Self, MeteringError<Meter>)> {
        let hash = self.inner.finalize();
        Ok(hash.into())
    }

    /// Computes the hash of the given data. Performs the same operation as [`Digest::digest`] but charges gas.
    ///
    /// # Errors
    /// Returns an error if charging gas for the hashing operation fails.
    pub fn digest(data: &[u8], meter: &'a mut Meter) -> Result<[u8; 32], MeteringError<Meter>> {
        let mut hasher = Self::new(meter);
        hasher.update(data)?;
        Self::finalize(hasher).map_err(|(_, e)| e)
    }
}

/// Representation of a signature verification error.
#[derive(Debug, thiserror::Error)]
pub enum MeteredSigVerificationError<GU: Gas> {
    /// The signature is invalid for the provided public key.
    #[error("Invalid signature: {0}")]
    BadSignature(SigVerificationError),

    /// There is not enough gas to verify the signature.
    #[error("A gas error was raised when trying to verify the signature, {0}")]
    GasError(GasMeteringError<GU>),
}

/// A metered signature that charges gas for signature verification. This is a wrapper around a [`Signature`] struct.
#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(bound = "GU: serde::Serialize + DeserializeOwned")]
pub struct MeteredSignature<GU: Gas, Sign: Signature> {
    inner: Sign,
    gas_to_charge_per_byte_for_verification: GU,
    fixed_gas_to_charge_per_verification: GU,
}

impl<GU: Gas, Sign: Signature> MeteredSignature<GU, Sign> {
    /// Creates a new [`MeteredSignature`] from a given [`Signature`] with a default gas price.
    pub fn new<Spec: GasSpec<Gas = GU>>(inner: Sign) -> Self {
        Self {
            inner,
            gas_to_charge_per_byte_for_verification:
                Spec::gas_to_charge_per_byte_signature_verification(),
            fixed_gas_to_charge_per_verification:
                Spec::fixed_gas_to_charge_per_signature_verification(),
        }
    }

    /// Creates a new [`MeteredSignature`] from a given [`Signature`] and a gas price.
    pub fn new_with_price(
        inner: Sign,
        fixed_gas_to_charge_per_signature: GU,
        gas_to_charge_per_byte_for_signature: GU,
    ) -> Self {
        Self {
            inner,
            fixed_gas_to_charge_per_verification: fixed_gas_to_charge_per_signature,
            gas_to_charge_per_byte_for_verification: gas_to_charge_per_byte_for_signature,
        }
    }

    /// Verifies a signature with the provided gas meter. This method is a wrapper around [`Signature::verify`].
    ///
    /// # Errors
    /// Returns an error if charging gas for the verification operation fails.
    pub fn verify<Meter: GasMeter<Spec: Spec<Gas = GU>>>(
        &self,
        pub_key: &Sign::PublicKey,
        msg: &[u8],
        meter: &mut Meter,
    ) -> Result<(), MeteredSigVerificationError<GU>> {
        meter
            .charge_gas(&self.fixed_gas_to_charge_per_verification)
            .map_err(MeteredSigVerificationError::GasError)?;

        meter
            .charge_linear_gas(
                &self.gas_to_charge_per_byte_for_verification,
                as_u32_or_panic(msg.len()),
            )
            .map_err(MeteredSigVerificationError::GasError)?;

        meter
            .charge_gas(&<Meter::Spec as GasSpec>::gas_to_charge_hash_update())
            .map_err(MeteredSigVerificationError::GasError)?;

        meter
            .charge_linear_gas(
                &<Meter::Spec as GasSpec>::gas_to_charge_per_byte_hash_update(),
                msg.len().try_into().map_err(|e: TryFromIntError| {
                    MeteredSigVerificationError::GasError(MeteringError::<Meter>::Overflow(
                        e.to_string(),
                    ))
                })?,
            )
            .map_err(MeteredSigVerificationError::GasError)?;

        self.inner
            .verify(pub_key, msg)
            .map_err(MeteredSigVerificationError::BadSignature)
    }
}

/// Representation of a metered borsh deserialization error.
#[derive(Debug, Error)]
pub enum MeteredBorshDeserializeError<GU: Gas> {
    /// A gas error was raised when trying to deserialize the data.
    #[error("A gas error was raised when trying to deserialize the data, {0}")]
    GasError(GasMeteringError<GU>),
    /// An io error occurred while deserializing the data.
    #[error("IO error: {0}")]
    IOError(io::Error),
}

/// Charges gas for deserialization.
pub trait MeteredBorshDeserialize<S: Spec>: Sized {
    /// The gas cost bias to deserialize this data structure.
    fn bias_borsh_deserialization() -> <S as Spec>::Gas;

    /// The linear gas cost to deserialize this data structure.
    fn gas_to_charge_per_byte_borsh_deserialization() -> <S as Spec>::Gas;

    /// Computes the cost to deserialize the given buffer, in `Gas`, and charges it to the provided
    /// `GasMeter`.
    ///
    /// # Errors
    /// Returns an error if charging the gas for the deserialization operation fails.
    fn charge_gas_to_deserialize(
        buf: &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<(), MeteredBorshDeserializeError<<S as GasSpec>::Gas>> {
        // This is safe to cast here. We won't have data bigger thane 4GB.
        let buf_len: u32 = as_u32_or_panic(buf.len());

        // Custom gas costs to deserialize this data structure.
        meter
            .charge_gas(&Self::bias_borsh_deserialization())
            .map_err(MeteredBorshDeserializeError::GasError)?;

        meter
            .charge_linear_gas(
                &Self::gas_to_charge_per_byte_borsh_deserialization(),
                buf_len,
            )
            .map_err(MeteredBorshDeserializeError::GasError)?;

        // Common gas costs to deserialize this data structure.
        meter
            .charge_gas(&S::bias_borsh_deserialization())
            .map_err(MeteredBorshDeserializeError::GasError)?;

        meter
            .charge_linear_gas(&S::gas_to_charge_per_byte_borsh_deserialization(), buf_len)
            .map_err(MeteredBorshDeserializeError::GasError)
    }

    /// Deserializes a type from a byte slice with the provided gas meter. Charge the [`GasSpec::gas_to_charge_per_byte_borsh_deserialization`]
    /// amount of gas for each byte of the struct to deserialize.
    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>>;

    #[cfg(feature = "native")]
    /// Deserialized a type without charging gas.
    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<Self, MeteredBorshDeserializeError<<S as GasSpec>::Gas>>;
}

/// Calculates `CredentialId`
pub fn metered_credential<S: Spec>(
    pub_key: &<S::CryptoSpec as CryptoSpec>::PublicKey,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<CredentialId, GasMeteringError<S::Gas>> {
    let cost = S::gas_to_charge_for_credential();
    meter.charge_gas(&cost)?;
    Ok(pub_key.credential_id())
}
