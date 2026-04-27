use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::capabilities::FatalError;
use sov_modules_api::transaction::{v1::MAX_SIGNERS, PubKeyAndSignature};
use sov_modules_api::SafeVec;
use sov_modules_api::{CryptoSpec, Spec};

use super::UnpackedSolanaMessage;

/// Discriminator byte prepended to the wire message in the multisig simple format.
/// Used only for borsh-level format routing (to distinguish multisig from single-sig envelopes).
/// This byte is `0x80`, which cannot occur as the first byte of valid UTF-8 (and therefore JSON),
/// nor does it collide with the spec-compliant preamble's discriminator.
/// It is NOT part of the signed content — signers sign the JSON payload directly.
pub const MULTISIG_SIMPLE_DISCRIMINATOR: u8 = 0x80;

/// The envelope for a message signed "raw", without the preamble included.
/// The preamble always starts with the \xff byte, whereas our raw message is JSON and so can only
/// start with an ASCII character (normally, '{'), allowing us to unambiguously differentiate them.
/// Without the preamble present, we need to include the pubkey explicitly.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSimpleMessage<S: Spec> {
    /// The message is a JSON-serialized SolanaOffchainUnsignedTransaction, unaltered.
    pub signed_message: Vec<u8>,
    pub chain_hash: [u8; 32],
    pub pubkey: <S::CryptoSpec as CryptoSpec>::PublicKey,
    pub signature: <S::CryptoSpec as CryptoSpec>::Signature,
}

/// The envelope for a multisig "simple" (preamble-less) solana offchain message.
/// Each signer independently signs the JSON payload (the bytes after the discriminator prefix).
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSimpleMultisigMessage<S: Spec> {
    /// Wire format: `[0x80][JSON-serialized SolanaOffchainUnsignedTransactionV1]`.
    /// The `0x80` prefix is a parsing discriminator only — the signed content is the JSON
    /// portion (everything after the first byte).
    pub wire_bytes: Vec<u8>,
    /// The chain hash at time of signing.
    pub chain_hash: [u8; 32],
    /// Signatures with their corresponding public keys (the signers who actually signed).
    #[borsh(bound(
        serialize = "PubKeyAndSignature<S::CryptoSpec>: BorshSerialize",
        deserialize = "PubKeyAndSignature<S::CryptoSpec>: BorshDeserialize",
    ))]
    pub signatures: SafeVec<PubKeyAndSignature<S::CryptoSpec>, MAX_SIGNERS>,
    /// Public keys that are part of the multisig but did not sign this transaction.
    #[borsh(bound(
        serialize = "<S::CryptoSpec as CryptoSpec>::PublicKey: BorshSerialize",
        deserialize = "<S::CryptoSpec as CryptoSpec>::PublicKey: BorshDeserialize",
    ))]
    pub unused_pub_keys: SafeVec<<S::CryptoSpec as CryptoSpec>::PublicKey, MAX_SIGNERS>,
    /// Minimum number of signers required for the multisig (the K in K-of-N).
    pub min_signers: u8,
}

pub(super) fn unpack_simple_message<S: Spec>(
    raw_tx: &[u8],
) -> Result<UnpackedSolanaMessage<S>, FatalError> {
    let raw_message: SolanaOffchainSimpleMessage<S> =
        borsh::from_slice(raw_tx).map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    Ok(UnpackedSolanaMessage::V0 {
        pub_key: raw_message.pubkey,
        signature: raw_message.signature,
        chain_hash: raw_message.chain_hash,
        signed_bytes: raw_message.signed_message,
        json_start: 0,
    })
}

pub(super) fn unpack_multisig_simple_message<S: Spec>(
    raw_tx: &[u8],
) -> Result<UnpackedSolanaMessage<S>, FatalError> {
    let msg: SolanaOffchainSimpleMultisigMessage<S> =
        borsh::from_slice(raw_tx).map_err(|e| FatalError::DeserializationFailed(e.to_string()))?;

    if msg.wire_bytes.first() != Some(&MULTISIG_SIMPLE_DISCRIMINATOR) {
        return Err(FatalError::DeserializationFailed(
            "Multisig simple message must start with 0x80 discriminator".to_string(),
        ));
    }

    // The signed content is the JSON payload after the discriminator prefix.
    let signed_bytes = msg.wire_bytes[1..].to_vec();

    Ok(UnpackedSolanaMessage::V1 {
        signatures: msg.signatures,
        unused_pub_keys: msg.unused_pub_keys,
        min_signers: msg.min_signers,
        chain_hash: msg.chain_hash,
        signed_bytes,
        json_start: 0,
    })
}
