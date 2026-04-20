use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::capabilities::FatalError;
use sov_modules_api::transaction::{v1::MAX_SIGNERS, PubKeyAndSignature};
use sov_modules_api::SafeVec;
use sov_modules_api::{CryptoSpec, Spec};

use super::UnpackedSolanaMessage;

/// The first byte of a spec-compliant preamble (`\xffsolana offchain`), used to detect
/// spec-compliant messages.
pub const SPEC_COMPLIANT_DISCRIMINATOR: u8 = 0xff;

pub(crate) mod format_constants {
    /// Length of the Solana offchain signing domain field.
    pub const SIGNING_DOMAIN_LEN: usize = 16;

    /// Length of the preamble header-version field.
    pub const HEADER_VERSION_LEN: usize = 1;

    /// Length of the application-domain field.
    pub const APPLICATION_DOMAIN_LEN: usize = 32;

    /// Length of the message-format field.
    pub const MESSAGE_FORMAT_LEN: usize = 1;

    /// Length of the signer-count field.
    pub const SIGNER_COUNT_LEN: usize = 1;

    /// Solana Ed25519 public keys are 32 bytes.
    pub const PUBKEY_LEN: usize = 32;

    /// Length of the message-length field.
    pub const MESSAGE_LENGTH_LEN: usize = 2;

    /// The fixed-size portion of the Solana offchain preamble, excluding the signer pubkey bytes.
    pub const PREAMBLE_FIXED_LEN: usize = SIGNING_DOMAIN_LEN
        + HEADER_VERSION_LEN
        + APPLICATION_DOMAIN_LEN
        + MESSAGE_FORMAT_LEN
        + SIGNER_COUNT_LEN
        + MESSAGE_LENGTH_LEN;
}

use self::format_constants::*;

// Derived offsets within the preamble (cumulative).
const HEADER_VERSION_OFFSET: usize = SIGNING_DOMAIN_LEN;
const APPLICATION_DOMAIN_OFFSET: usize = HEADER_VERSION_OFFSET + HEADER_VERSION_LEN;
const MESSAGE_FORMAT_OFFSET: usize = APPLICATION_DOMAIN_OFFSET + APPLICATION_DOMAIN_LEN;
pub(super) const SIGNER_COUNT_OFFSET: usize = MESSAGE_FORMAT_OFFSET + MESSAGE_FORMAT_LEN;
const SIGNERS_START: usize = SIGNER_COUNT_OFFSET + SIGNER_COUNT_LEN;

/// The envelope for a signed spec-compliant solana offchain message, where the signed message
/// includes the preamble.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSpecCompliantMessage<S: Spec> {
    /// The message is a JSON-serialized SolanaOffchainUnsignedTransaction with the standard
    /// preamble prepended.
    pub signed_message_with_preamble: Vec<u8>,
    pub signature: <S::CryptoSpec as CryptoSpec>::Signature,
}

/// The envelope for a multisig spec-compliant solana offchain message.
/// All pubkeys are embedded in the preamble (part of the signed bytes). The envelope carries only
/// signatures, a bitfield mapping each signature to its pubkey in the preamble, and the threshold.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSpecCompliantMultisigMessage<S: Spec> {
    /// Preamble (with all N pubkeys) followed by JSON-serialized
    /// `SolanaOffchainUnsignedTransactionV1`.
    pub signed_message_with_preamble: Vec<u8>,
    /// One signature per signer, ordered to match set bits in `signer_bitfield` from LSB to MSB.
    #[borsh(bound(
        serialize = "<S::CryptoSpec as CryptoSpec>::Signature: BorshSerialize",
        deserialize = "<S::CryptoSpec as CryptoSpec>::Signature: BorshDeserialize",
    ))]
    pub signatures: SafeVec<<S::CryptoSpec as CryptoSpec>::Signature, MAX_SIGNERS>,
    /// Bitfield marking which pubkeys in the preamble have corresponding signatures.
    /// Bit `i` (0-indexed from LSB) = 1 means the `i`-th preamble pubkey signed.
    pub signer_bitfield: u32,
    /// Minimum number of signers required for the multisig (the K in K-of-N).
    pub min_signers: u8,
}

/// The preamble/header required for signing solana offchain messages, see above for spec link.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct RawSolanaOffchainMessagePreamble {
    pub signing_domain: [u8; SIGNING_DOMAIN_LEN],
    pub header_version: u8,
    pub application_domain: [u8; APPLICATION_DOMAIN_LEN],
    pub message_format: u8,
    pub signer_count: u8,
    pub signer: [u8; PUBKEY_LEN],
    pub message_length: [u8; MESSAGE_LENGTH_LEN],
}

impl RawSolanaOffchainMessagePreamble {
    /// Validates a Solana offchain message preamble.
    fn validate(&self, actual_message_length: usize) -> Result<(), FatalError> {
        if self.signing_domain != *b"\xffsolana offchain" {
            return Err(FatalError::DeserializationFailed(
                "Invalid Solana signing domain in preamble".to_string(),
            ));
        }
        // 0 and 1 are the supported header versions.
        // Version 1 added newline support to ASCII messages, and is otherwise identical.
        if self.header_version != 0 && self.header_version != 1 {
            return Err(FatalError::DeserializationFailed(format!(
                "Invalid header version in preamble: only versions 0 and 1 are supported, but version {} was provided",
                self.header_version
            )));
        }
        // Format 0 is the ASCII, hw-wallet compatible format.
        if self.message_format != 0 {
            return Err(FatalError::DeserializationFailed(format!(
                "Invalid message format in preamble: only format 0 is supported, but format {} was provided",
                self.message_format
            )));
        }
        if self.signer_count != 1 {
            return Err(FatalError::DeserializationFailed(format!(
                "Invalid signer count in preamble: only a single signer is currently supported, but the count was {}",
                self.signer_count
            )));
        }
        let expected_length = u16::from_le_bytes(self.message_length) as usize;
        if expected_length != actual_message_length {
            return Err(FatalError::DeserializationFailed(format!(
                "Message length mismatch: expected {expected_length}, got {actual_message_length}"
            )));
        }

        Ok(())
    }
}

/// Computes the total preamble length for `signer_count` signers.
fn preamble_len(signer_count: u8) -> usize {
    PREAMBLE_FIXED_LEN + PUBKEY_LEN * signer_count as usize
}

/// Parsed result of a multisig preamble (signer_count >= 2).
struct ParsedMultisigPreamble {
    application_domain: [u8; 32],
    signers: Vec<[u8; 32]>,
}

/// Parses a Solana offchain message preamble with multiple signers.
/// Validates all fields and returns the parsed data.
fn parse_and_validate_multisig_preamble(
    data: &[u8],
    actual_message_length: usize,
) -> Result<ParsedMultisigPreamble, FatalError> {
    let min_multisig_preamble = preamble_len(2);
    if data.len() < min_multisig_preamble {
        return Err(FatalError::DeserializationFailed(
            "Preamble too short for multisig".to_string(),
        ));
    }

    if data[0..SIGNING_DOMAIN_LEN] != *b"\xffsolana offchain" {
        return Err(FatalError::DeserializationFailed(
            "Invalid Solana signing domain in preamble".to_string(),
        ));
    }

    let header_version = data[HEADER_VERSION_OFFSET];
    if header_version != 0 && header_version != 1 {
        return Err(FatalError::DeserializationFailed(format!(
            "Invalid header version in preamble: only versions 0 and 1 are supported, but version {header_version} was provided"
        )));
    }

    let application_domain_end = APPLICATION_DOMAIN_OFFSET + APPLICATION_DOMAIN_LEN;
    let application_domain: [u8; APPLICATION_DOMAIN_LEN] = data
        [APPLICATION_DOMAIN_OFFSET..application_domain_end]
        .try_into()
        .unwrap();

    let message_format = data[MESSAGE_FORMAT_OFFSET];
    if message_format != 0 {
        return Err(FatalError::DeserializationFailed(format!(
            "Invalid message format in preamble: only format 0 is supported, but format {message_format} was provided"
        )));
    }

    let signer_count = data[SIGNER_COUNT_OFFSET];
    if signer_count < 2 || signer_count as usize > MAX_SIGNERS {
        return Err(FatalError::DeserializationFailed(format!(
            "Invalid signer count in multisig preamble: expected 2..={MAX_SIGNERS}, got {signer_count}"
        )));
    }

    let total_preamble = preamble_len(signer_count);
    if data.len() < total_preamble {
        return Err(FatalError::DeserializationFailed(format!(
            "Preamble too short: need {total_preamble} bytes for {signer_count} signers, got {}",
            data.len()
        )));
    }

    let mut signers = Vec::with_capacity(signer_count as usize);
    for i in 0..signer_count as usize {
        let start = SIGNERS_START + i * PUBKEY_LEN;
        let signer: [u8; PUBKEY_LEN] = data[start..start + PUBKEY_LEN].try_into().unwrap();
        signers.push(signer);
    }

    let msg_len_offset = SIGNERS_START + signer_count as usize * PUBKEY_LEN;
    let message_length = u16::from_le_bytes(
        data[msg_len_offset..msg_len_offset + MESSAGE_LENGTH_LEN]
            .try_into()
            .unwrap(),
    );

    if message_length as usize != actual_message_length {
        return Err(FatalError::DeserializationFailed(format!(
            "Message length mismatch: expected {message_length}, got {actual_message_length}"
        )));
    }

    Ok(ParsedMultisigPreamble {
        application_domain,
        signers,
    })
}

type SignaturesAndUnusedKeys<C> = (
    SafeVec<PubKeyAndSignature<C>, MAX_SIGNERS>,
    SafeVec<<C as CryptoSpec>::PublicKey, MAX_SIGNERS>,
);

/// Uses the signer bitfield to pair each signature with its corresponding preamble pubkey,
/// and collects the remaining pubkeys as unused.
fn pair_signatures_with_preamble_pubkeys<S: Spec>(
    preamble_signers: &[[u8; 32]],
    signatures: &SafeVec<<S::CryptoSpec as CryptoSpec>::Signature, MAX_SIGNERS>,
    signer_bitfield: u32,
) -> Result<SignaturesAndUnusedKeys<S::CryptoSpec>, FatalError> {
    let signer_count = preamble_signers.len();

    // Validate bitfield: no out-of-range bits set.
    let valid_bits_mask = (1u32 << signer_count) - 1;
    if signer_bitfield & !valid_bits_mask != 0 {
        return Err(FatalError::DeserializationFailed(format!(
            "Signer bitfield has bits set beyond signer_count ({signer_count})"
        )));
    }

    // Validate bitfield popcount matches number of signatures.
    let expected_sig_count = signer_bitfield.count_ones() as usize;
    if expected_sig_count != signatures.len() {
        return Err(FatalError::DeserializationFailed(format!(
            "Signer bitfield popcount ({expected_sig_count}) doesn't match signature count ({})",
            signatures.len()
        )));
    }

    let mut paired = Vec::with_capacity(expected_sig_count);
    let mut unused = Vec::with_capacity(signer_count - expected_sig_count);
    let mut sig_idx = 0;

    for (i, signer_bytes) in preamble_signers.iter().enumerate() {
        let pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey = borsh::from_slice(signer_bytes)
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

        if signer_bitfield & (1 << i) != 0 {
            paired.push(PubKeyAndSignature {
                signature: signatures[sig_idx].clone(),
                pub_key,
            });
            sig_idx += 1;
        } else {
            unused.push(pub_key);
        }
    }

    let paired = paired
        .try_into()
        .map_err(|_| FatalError::DeserializationFailed("Too many signatures".to_string()))?;
    let unused = unused
        .try_into()
        .map_err(|_| FatalError::DeserializationFailed("Too many unused pubkeys".to_string()))?;
    Ok((paired, unused))
}

pub(super) fn unpack_spec_compliant_message<S: Spec>(
    raw_tx: &[u8],
) -> Result<UnpackedSolanaMessage<S>, FatalError> {
    let single_key_preamble_len = preamble_len(1);

    let envelope: SolanaOffchainSpecCompliantMessage<S> =
        borsh::from_slice(raw_tx).map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    if envelope.signed_message_with_preamble.len() < single_key_preamble_len {
        return Err(FatalError::DeserializationFailed(
            "Message too short for preamble".to_string(),
        ));
    }

    let preamble: RawSolanaOffchainMessagePreamble =
        borsh::from_slice(&envelope.signed_message_with_preamble[0..single_key_preamble_len])
            .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    // Unwrap: we just checked the length is >= single_key_preamble_len, so this can't underflow.
    let actual_message_length = envelope
        .signed_message_with_preamble
        .len()
        .checked_sub(single_key_preamble_len)
        .unwrap();
    preamble.validate(actual_message_length)?;

    let signer: <S::CryptoSpec as CryptoSpec>::PublicKey = borsh::from_slice(&preamble.signer)
        .map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    Ok(UnpackedSolanaMessage::V0 {
        pub_key: signer,
        signature: envelope.signature,
        chain_hash: preamble.application_domain,
        signed_bytes: envelope.signed_message_with_preamble,
        json_start: single_key_preamble_len,
    })
}

pub(super) fn unpack_spec_compliant_multisig_message<S: Spec>(
    raw_tx: &[u8],
) -> Result<UnpackedSolanaMessage<S>, FatalError> {
    let envelope: SolanaOffchainSpecCompliantMultisigMessage<S> =
        borsh::from_slice(raw_tx).map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    let data = &envelope.signed_message_with_preamble;
    if data.len() < (SIGNER_COUNT_OFFSET + 1) {
        return Err(FatalError::DeserializationFailed(
            "Message too short for multisig preamble".to_string(),
        ));
    }

    let signer_count = data[SIGNER_COUNT_OFFSET];
    let preamble_size = preamble_len(signer_count);
    if data.len() < preamble_size {
        return Err(FatalError::DeserializationFailed(format!(
            "Message too short for preamble with {signer_count} signers"
        )));
    }

    let actual_message_length = data.len() - preamble_size;
    let preamble =
        parse_and_validate_multisig_preamble(&data[..preamble_size], actual_message_length)?;

    let (signatures, unused_pub_keys) = pair_signatures_with_preamble_pubkeys::<S>(
        &preamble.signers,
        &envelope.signatures,
        envelope.signer_bitfield,
    )?;

    Ok(UnpackedSolanaMessage::V1 {
        signatures,
        unused_pub_keys,
        min_signers: envelope.min_signers,
        chain_hash: preamble.application_domain,
        signed_bytes: envelope.signed_message_with_preamble,
        json_start: preamble_size,
    })
}
