pub(super) mod simple;
pub(super) mod spec_compliant;

use sov_modules_api::capabilities::FatalError;
use sov_modules_api::transaction::{v1::MAX_SIGNERS, PubKeyAndSignature};
use sov_modules_api::SafeVec;
use sov_modules_api::{CryptoSpec, Spec};

#[derive(Debug)]
pub(super) enum UnpackedSolanaMessage<S: Spec> {
    V0 {
        pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        signature: <S::CryptoSpec as CryptoSpec>::Signature,
        chain_hash: [u8; 32],
        signed_bytes: Vec<u8>,
        json_start: usize,
    },
    V1 {
        signatures: SafeVec<PubKeyAndSignature<S::CryptoSpec>, MAX_SIGNERS>,
        unused_pub_keys: SafeVec<<S::CryptoSpec as CryptoSpec>::PublicKey, MAX_SIGNERS>,
        min_signers: u8,
        chain_hash: [u8; 32],
        signed_bytes: Vec<u8>,
        json_start: usize,
    },
}

impl<S: Spec> UnpackedSolanaMessage<S> {
    pub(super) fn json_bytes(&self) -> &[u8] {
        match self {
            UnpackedSolanaMessage::V0 {
                signed_bytes,
                json_start,
                ..
            }
            | UnpackedSolanaMessage::V1 {
                signed_bytes,
                json_start,
                ..
            } => &signed_bytes[*json_start..],
        }
    }

    pub(super) fn chain_hash(&self) -> &[u8; 32] {
        match self {
            UnpackedSolanaMessage::V0 { chain_hash, .. }
            | UnpackedSolanaMessage::V1 { chain_hash, .. } => chain_hash,
        }
    }

    pub(super) fn signed_bytes(&self) -> &[u8] {
        match self {
            UnpackedSolanaMessage::V0 { signed_bytes, .. }
            | UnpackedSolanaMessage::V1 { signed_bytes, .. } => signed_bytes,
        }
    }
}

pub(super) fn unpack_solana_message<S: Spec>(
    raw_tx: &[u8],
) -> Result<UnpackedSolanaMessage<S>, FatalError> {
    // First 4 bytes are the length of the Vec<u8> as u32 (borsh encoding)
    const BORSH_VEC_LEN_PREFIX: usize = 4;
    if raw_tx.len() < BORSH_VEC_LEN_PREFIX + 1 {
        return Err(FatalError::DeserializationFailed(
            "Message too short".to_string(),
        ));
    }

    // The fifth byte tells us which format we're dealing with:
    // 0xff → Spec-compliant message (preamble starts with \xffsolana offchain)
    // 0x80 → Multisig simple message (discriminator prefix)
    // Anything else → Simple message (JSON, typically starts with '{')
    if raw_tx[BORSH_VEC_LEN_PREFIX] == spec_compliant::SPEC_COMPLIANT_DISCRIMINATOR {
        // signer_count sits after the borsh Vec<u8> length prefix plus the preamble offset.
        const RAW_TX_SIGNER_COUNT_OFFSET: usize =
            BORSH_VEC_LEN_PREFIX + spec_compliant::SIGNER_COUNT_OFFSET;
        if raw_tx.len() <= RAW_TX_SIGNER_COUNT_OFFSET {
            return Err(FatalError::DeserializationFailed(
                "Message too short to read signer count".to_string(),
            ));
        }
        if raw_tx[RAW_TX_SIGNER_COUNT_OFFSET] > 1 {
            spec_compliant::unpack_spec_compliant_multisig_message(raw_tx)
        } else {
            spec_compliant::unpack_spec_compliant_message(raw_tx)
        }
    } else if raw_tx[BORSH_VEC_LEN_PREFIX] == simple::MULTISIG_SIMPLE_DISCRIMINATOR {
        simple::unpack_multisig_simple_message(raw_tx)
    } else {
        simple::unpack_simple_message(raw_tx)
    }
}
