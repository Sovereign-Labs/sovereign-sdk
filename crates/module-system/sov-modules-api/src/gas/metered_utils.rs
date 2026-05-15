use std::io;
use std::num::TryFromIntError;

use digest::consts::U32;
use digest::Digest;
use serde::de::DeserializeOwned;
use sov_rollup_interface::crypto::{CredentialId, SigVerificationError, Signature};
use thiserror::Error;

use crate::gas::traits::{Gas, GasMeter};
use crate::{as_u32_or_panic, CryptoSpecExt, GasMeteringError, GasSpec, PublicKey, Spec};

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
        self.meter.charge_gas(self.gas_to_charge_for_hash_update)?;
        self.meter.charge_linear_gas(
            self.gas_to_charge_per_byte_for_hash_update,
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

    /// Charges gas for the signature verification.
    pub fn charge_gas<Meter: GasMeter<Spec: Spec<Gas = GU>>>(
        &self,
        meter: &mut Meter,
        msg_len: usize,
    ) -> Result<(), MeteredSigVerificationError<GU>> {
        charge_gas_for_sig_inner(
            meter,
            msg_len,
            self.fixed_gas_to_charge_per_verification,
            self.gas_to_charge_per_byte_for_verification,
        )
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
        self.charge_gas(meter, msg.len())?;

        self.inner
            .verify(pub_key, msg)
            .map_err(MeteredSigVerificationError::BadSignature)
    }
}

/// Charge gas for transaction signature verification.
pub fn charge_gas_for_sig<S: Spec, Meter: GasMeter<Spec = S>>(
    meter: &mut Meter,
    msg_len: usize,
) -> Result<(), MeteredSigVerificationError<S::Gas>> {
    charge_gas_for_sig_inner(
        meter,
        msg_len,
        S::fixed_gas_to_charge_per_signature_verification(),
        S::gas_to_charge_per_byte_signature_verification(),
    )
}

fn charge_gas_for_sig_inner<GU: Gas, Meter: GasMeter<Spec: Spec<Gas = GU>>>(
    meter: &mut Meter,
    msg_len: usize,
    fixed_gas_to_charge_per_signature: GU,
    gas_to_charge_per_byte_for_signature: GU,
) -> Result<(), MeteredSigVerificationError<GU>> {
    meter
        .charge_gas(fixed_gas_to_charge_per_signature)
        .map_err(MeteredSigVerificationError::GasError)?;

    meter
        .charge_linear_gas(
            gas_to_charge_per_byte_for_signature,
            as_u32_or_panic(msg_len),
        )
        .map_err(MeteredSigVerificationError::GasError)?;

    Ok(())
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

/// Extension trait that charges gas for borsh deserialization as the decoder
/// actually consumes bytes. Auto-implemented for every `T: borsh::BorshDeserialize`
/// via the blanket impl below.
///
/// The gas spec comes from the meter passed into each method — the trait itself
/// has no `Spec` parameter, so callers don't need to disambiguate it.
pub trait MeteredBorshDeserialize: Sized + borsh::BorshDeserialize {
    /// Decode `Self` from a metered reader. Every read against the reader charges
    /// gas, so total cost tracks actual decode work rather than the input length.
    fn deserialize_reader<R: io::Read, M: GasMeter>(
        reader: &mut crate::MeteredReader<'_, R, M>,
    ) -> Result<Self, MeteredBorshDeserializeError<<M::Spec as Spec>::Gas>> {
        <Self as borsh::BorshDeserialize>::deserialize_reader(reader)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    /// Slice-driven entry point. Charges [`GasSpec::bias_borsh_deserialization`],
    /// wraps `buf` in a [`crate::MeteredReader`], delegates to `deserialize_reader`,
    /// advances `*buf` by the bytes consumed, and recovers stashed gas errors.
    fn deserialize_from_slice<M: GasMeter>(
        buf: &mut &[u8],
        meter: &mut M,
    ) -> Result<Self, MeteredBorshDeserializeError<<M::Spec as Spec>::Gas>> {
        meter
            .charge_gas(<M::Spec as GasSpec>::bias_borsh_deserialization())
            .map_err(MeteredBorshDeserializeError::GasError)?;

        let mut cursor = io::Cursor::new(*buf);
        let mut reader = crate::MeteredReader::new(&mut cursor, meter);
        let result = <Self as MeteredBorshDeserialize>::deserialize_reader(&mut reader);
        match result {
            Ok(value) => {
                let consumed = cursor.position() as usize;
                *buf = &buf[consumed..];
                Ok(value)
            }
            Err(MeteredBorshDeserializeError::IOError(io_err)) => match reader.take_stashed_error()
            {
                Some(gas_err) => Err(MeteredBorshDeserializeError::GasError(gas_err)),
                None => Err(MeteredBorshDeserializeError::IOError(io_err)),
            },
            Err(other) => Err(other),
        }
    }

    #[cfg(feature = "native")]
    /// Deserialize without charging gas. Native-only escape hatch for places that
    /// already paid for the work (e.g. CLI, test scaffolding).
    fn unmetered_deserialize(buf: &mut &[u8]) -> Result<Self, io::Error> {
        <Self as borsh::BorshDeserialize>::deserialize(buf)
    }
}

impl<T: borsh::BorshDeserialize> MeteredBorshDeserialize for T {}

/// Computes the cost to deserialize the given JSON buffer, in `Gas`, and charges it to the provided
/// `GasMeter`.
///
/// # Errors
/// Returns an error if charging the gas for the deserialization operation fails.
pub fn charge_gas_to_deserialize_json<S: Spec>(
    buf: &[u8],
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), GasMeteringError<<S as GasSpec>::Gas>> {
    // This is safe to cast here. We won't have data bigger than 4GB.
    let buf_len: u32 = as_u32_or_panic(buf.len());

    // Custom gas costs to deserialize this data structure.
    meter.charge_gas(S::tx_bias_json_deserialization())?;

    meter.charge_linear_gas(S::tx_gas_to_charge_per_byte_json_deserialization(), buf_len)?;

    // Since JSON is not used often, no common cost to JSON deserialization is defined to
    // simplify the set of constants.

    Ok(())
}

/// Calculates `CredentialId`
pub fn metered_credential<S: Spec, C: CryptoSpecExt>(
    pub_key: &C::PublicKey,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<CredentialId, GasMeteringError<S::Gas>> {
    let cost = S::gas_to_charge_for_credential();
    meter.charge_gas(cost)?;
    Ok(pub_key.credential_id())
}
