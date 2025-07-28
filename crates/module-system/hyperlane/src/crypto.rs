//! Cryptographic primitves / helpers used in hyperlane

use anyhow::{ensure, Context, Result};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};
use sov_modules_api::digest::FixedOutput;
use sov_modules_api::{GasArray, GasMeter, GasSpec, HexHash, HexString, Spec};

use crate::types::{EthAddress, Message, StorageLocation};

/// The EIP-191 prefix for an Ethereum signed message
/// <https://eips.ethereum.org/EIPS/eip-191>
pub const ETH_SIGNED_MESSAGE_PREFIX: &str = "\x19Ethereum Signed Message:\n";

/// Computes the hash of the message that was used to construct the validators' signatures.
/// Note that this is *not* the simple keccak256 hash of the `Message` struct. A reference implementation can be found here:
/// <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/libs/CheckpointLib.sol#L28>
pub fn compute_hash_for_signatures<S: Spec>(
    message: &Message,
    origin_merkle_tree: &HexHash,
    merkle_root: &HexHash,
    merkle_index: u32,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<EthSignHash> {
    // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/libs/CheckpointLib.sol#L80
    let domain_hash = DomainHash::new(
        message.origin_domain,
        &origin_merkle_tree.0,
        HashKind::Hyperlane,
        gas_meter,
    )?;
    // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/libs/CheckpointLib.sol#L37
    let multisig_hash = MultisigHash::new(
        domain_hash,
        *merkle_root,
        merkle_index,
        message.id(gas_meter)?,
        gas_meter,
    )?;
    EthSignHash::new(multisig_hash.0, gas_meter)
}

/// Decodes [`RecoverableSignature`] out of a slice of bytes.
// See <https://github.com/eigerco/hyperlane-monorepo/blob/b68fe264b3585ecd9d95a5ec2ec2d7defbe907d2/rust/sealevel/libraries/ecdsa-signature/src/lib.rs#L40>
pub fn decode_signature(bytes: &[u8]) -> Result<RecoverableSignature> {
    ensure!(
        bytes.len() == 65,
        "Compact recoverable signature must be 65 bytes"
    );

    let mut recovery_id = bytes[64];
    if recovery_id == 27 || recovery_id == 28 {
        recovery_id -= 27;
    }
    if recovery_id > 1 {
        Err(anyhow::anyhow!("Invalid recovery id {}", recovery_id))?;
    }
    Ok(RecoverableSignature {
        // Safety: We've already checked that the bytes are 64 bytes long
        signature: Signature::from_bytes(bytes[..64].into())?,
        recovery_id: RecoveryId::from_byte(recovery_id)
            .expect("Recovery id was just checked to be valid but is now invalid"),
    })
}

/// A signature and recovery id
pub struct RecoverableSignature {
    signature: Signature,
    recovery_id: RecoveryId,
}

/// A 64 byte uncompressed ECDSA public key.
pub struct EcdsaPubKeyBytes(pub [u8; 64]);

impl From<VerifyingKey> for EcdsaPubKeyBytes {
    fn from(value: VerifyingKey) -> Self {
        // The first byte is the compression flag, which we don't care about.
        // https://stackoverflow.com/questions/66383584/how-to-extract-uncompressed-public-key-from-secp256k1
        let encoded = value.to_encoded_point(false);
        let bytes = &encoded.as_bytes()[1..];
        EcdsaPubKeyBytes(bytes.try_into().unwrap())
    }
}

/// Recover public key from message hash and signature
pub fn ec_recover<S: Spec>(
    digest: impl Into<[u8; 32]>,
    signature: &RecoverableSignature,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<EcdsaPubKeyBytes> {
    gas_meter.charge_gas(&<S as GasSpec>::fixed_gas_to_charge_per_signature_verification())?;

    let public_key = VerifyingKey::recover_from_prehash(
        &digest.into(),
        &signature.signature,
        signature.recovery_id,
    )?;
    Ok(public_key.into())
}

/// Derive ethereum address from public key
pub fn eth_address_from_public_key<S: Spec>(
    pub_key: impl Into<EcdsaPubKeyBytes>,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<EthAddress> {
    let pub_key = pub_key.into();
    let hash = keccak256_hash(&pub_key.0, gas_meter)?;
    // truncate first 12 bytes
    Ok(HexString(
        hash.0[12..].try_into().expect("Must be exactly 20 bytes"),
    ))
}

/// Convert a slice of bytes into a 32-byte hash using the keccak256 algorithm.
pub fn keccak256_hash<S: Spec>(
    bz: &[u8],
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<HexHash> {
    charge_gas_for_hashing(bz.len(), gas_meter)?;
    Ok(Keccak256::digest(bz).into())
}

/// Combine two hashes into a single one using keccak 256 algorithm.
pub fn keccak256_concat<S: Spec>(
    a: &HexHash,
    b: &HexHash,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<HexHash> {
    charge_gas_for_hashing(a.0.len() + b.0.len(), gas_meter)?;

    let mut hasher = Keccak256::new();
    hasher.update(a);
    hasher.update(b);
    Ok(hasher.finalize_fixed().into())
}

/// Domain separators for Hyperlane signed messages
pub enum HashKind {
    #[allow(dead_code)]
    /// The message is a validator "announcement"
    // Note: This will be used once we re-enable validator announcements
    HyperlaneAnnouncement,
    /// The message is a Hyperlane protocol message
    Hyperlane,
}

impl AsRef<[u8]> for HashKind {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::HyperlaneAnnouncement => "HYPERLANE_ANNOUNCEMENT".as_bytes(),
            Self::Hyperlane => "HYPERLANE".as_bytes(),
        }
    }
}

/// A domain hash is a hash of the domain ID, mailbox address, and one of set of well known domain separators
// See <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/libs/CheckpointLib.sol#L80>
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DomainHash(pub [u8; 32]);

impl DomainHash {
    /// Create a new domain hash
    pub fn new<S: Spec>(
        domain: u32,
        mailbox_addr: &[u8; 32],
        kind: HashKind,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self> {
        let domain = domain.to_be_bytes();
        let kind = kind.as_ref();
        charge_gas_for_hashing(domain.len() + mailbox_addr.len() + kind.len(), gas_meter)?;

        Ok(Self(
            Keccak256::new()
                .chain_update(domain)
                .chain_update(mailbox_addr)
                .chain_update(kind)
                .finalize()
                .into(),
        ))
    }
}

/// A hash used in signing messages for the `MultiSig` ISM
// https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/libs/CheckpointLib.sol#L37
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MultisigHash(pub [u8; 32]);

impl MultisigHash {
    /// Create a new multisig hash
    pub fn new<S: Spec>(
        domain_hash: DomainHash,
        merkle_root: HexHash,
        index: u32,
        message_id: HexHash,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self> {
        let index = index.to_be_bytes();
        charge_gas_for_hashing(
            domain_hash.0.len() + merkle_root.0.len() + index.len() + message_id.0.len(),
            gas_meter,
        )?;

        Ok(Self(
            Keccak256::new()
                .chain_update(domain_hash.0)
                .chain_update(merkle_root)
                .chain_update(index)
                .chain_update(message_id)
                .finalize()
                .into(),
        ))
    }
}

/// An ethereum-style hash - using a well known prefix, a length, and some data.
/// See <https://eips.ethereum.org/EIPS/eip-191>
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EthSignHash(pub [u8; 32]);

impl EthSignHash {
    /// Create a new eth signature hash
    pub fn new<S: Spec>(
        value: impl AsRef<[u8]>,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self> {
        let value = value.as_ref();
        let value_len = value.len().to_string();
        charge_gas_for_hashing(
            ETH_SIGNED_MESSAGE_PREFIX.len() + value_len.len() + value.len(),
            gas_meter,
        )?;
        Ok(Self(
            Keccak256::new()
                .chain_update(ETH_SIGNED_MESSAGE_PREFIX)
                .chain_update(value_len)
                .chain_update(value)
                .finalize()
                .into(),
        ))
    }
}

/// A hash used in signing messages for the validator announcements
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementHash(pub [u8; 32]);

impl AnnouncementHash {
    /// Create a new validator announcement hash
    pub fn new<S: Spec>(
        domain_hash: DomainHash,
        location: &StorageLocation,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self> {
        charge_gas_for_hashing(
            domain_hash.0.len() + <StorageLocation as AsRef<[u8]>>::as_ref(location).len(),
            gas_meter,
        )?;
        Ok(Self(
            Keccak256::new()
                .chain_update(domain_hash.0)
                .chain_update(location)
                .finalize()
                .into(),
        ))
    }
}

pub(crate) fn charge_gas_for_hashing<S: Spec>(
    bytes_to_hash: usize,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<()> {
    let gas_multiplier = <<<S as GasSpec>::Gas as GasArray>::Scalar>::try_from(bytes_to_hash)
        .context("Overflow creating scalar from amount of bytes to hash")?;

    gas_meter.charge_gas(&<S as GasSpec>::gas_to_charge_hash_update())?;
    gas_meter.charge_gas(
        &<S as GasSpec>::gas_to_charge_per_byte_hash_update()
            .checked_scalar_product(gas_multiplier)
            .context("Overflow calculating gas to charge")?,
    )?;

    Ok(())
}
