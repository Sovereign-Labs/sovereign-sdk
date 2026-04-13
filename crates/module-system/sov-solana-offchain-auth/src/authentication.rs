use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::capabilities::AuthorizationData;
use sov_modules_api::capabilities::{
    calculate_hash_metered, verify_chain_id, AuthenticationError, AuthenticationOutput, FatalError,
    UniquenessData,
};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::transaction::Credentials;
use sov_modules_api::transaction::{v1::MAX_SIGNERS, PubKeyAndSignature};
use sov_modules_api::transaction::{
    AuthenticatedTransactionAndRawHash, TransactionCallable, TxDetails, UnsignedTransaction,
};
use sov_modules_api::SafeVec;
use sov_modules_api::{
    charge_gas_to_deserialize_json, CryptoSpec, DispatchCall, GasMeter, GasSpec,
    MeteredSigVerificationError, MeteredSignature, Multisig, ProvableStateReader, SafeString,
    Signature, Spec, TxHash,
};

#[cfg(feature = "native")]
use sov_modules_api::capabilities::{SignatureVerificationCache, DEFAULT_SIGNATURE_CACHE_SIZE};

#[cfg(feature = "native")]
static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<()>> =
    std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));

/// The payload for a solana offchain message.
/// Essentially a wrapper around `sov_modules_api::transaction::UnsignedTransaction` that also
/// includes the chain_name, in order to ensure the name gets displayed to the user and signed as
/// part of the message.
/// We duplicate the UnsignedTransaction type rather than wrapping it to ensure the JSON displayed
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
    fn into_unsigned_tx(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
        }
    }

    fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
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
    fn into_unsigned_tx(self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction {
            runtime_call: self.runtime_call,
            uniqueness: self.uniqueness,
            details: self.details,
        }
    }

    fn unmetered_deserialize(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice::<SolanaOffchainUnsignedTransactionV1<R, S>>(buf)
    }
}

/// The envelope for a signed spec-compliant solana offchain message, where the signed message
/// includes the preamble.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSpecCompliantMessage<S: Spec> {
    /// The message is a JSON-serialized SolanaOffchainUnsignedTransaction with the standard preamble prepended.
    pub signed_message_with_preamble: Vec<u8>,
    pub signature: <S::CryptoSpec as CryptoSpec>::Signature,
}

/// The envelope for a message signed "raw", without the preable included.
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

/// The first byte of a spec-compliant preamble (`\xffsolana offchain`), used to detect spec-compliant messages.
pub const SPEC_COMPLIANT_DISCRIMINATOR: u8 = 0xff;

/// Discriminator byte prepended to the wire message in the multisig simple format.
/// Used only for borsh-level format routing (to distinguish multisig from single-sig envelopes).
/// This byte is `0x80`, which cannot occur as the first byte of valid UTF-8 (and therefore JSON),
/// nor does it collide with the spec-compliant preamble's [`SPEC_COMPLIANT_DISCRIMINATOR`].
/// It is NOT part of the signed content — signers sign the JSON payload directly.
pub const MULTISIG_SIMPLE_DISCRIMINATOR: u8 = 0x80;

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

/// The envelope for a multisig spec-compliant solana offchain message.
/// All pubkeys are embedded in the preamble (part of the signed bytes). The envelope carries only
/// signatures, a bitfield mapping each signature to its pubkey in the preamble, and the threshold.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SolanaOffchainSpecCompliantMultisigMessage<S: Spec> {
    /// Preamble (with all N pubkeys) followed by JSON-serialized `SolanaOffchainUnsignedTransactionV1`.
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

// Preamble field sizes, per the Solana offchain message spec.
/// See <https://docs.anza.xyz/proposals/off-chain-message-signing#message-preamble>
const SIGNING_DOMAIN_LEN: usize = 16;
const HEADER_VERSION_LEN: usize = 1;
const APPLICATION_DOMAIN_LEN: usize = 32;
const MESSAGE_FORMAT_LEN: usize = 1;
const SIGNER_COUNT_LEN: usize = 1;
pub(crate) const PUBKEY_LEN: usize = 32;
const MESSAGE_LENGTH_LEN: usize = 2;

/// The sum of all fixed-size preamble fields (everything except the variable-length signers array).
pub(crate) const PREAMBLE_FIXED_LEN: usize = SIGNING_DOMAIN_LEN
    + HEADER_VERSION_LEN
    + APPLICATION_DOMAIN_LEN
    + MESSAGE_FORMAT_LEN
    + SIGNER_COUNT_LEN
    + MESSAGE_LENGTH_LEN;

// Derived offsets within the preamble (cumulative).
const HEADER_VERSION_OFFSET: usize = SIGNING_DOMAIN_LEN;
const APPLICATION_DOMAIN_OFFSET: usize = HEADER_VERSION_OFFSET + HEADER_VERSION_LEN;
const MESSAGE_FORMAT_OFFSET: usize = APPLICATION_DOMAIN_OFFSET + APPLICATION_DOMAIN_LEN;
const SIGNER_COUNT_OFFSET: usize = MESSAGE_FORMAT_OFFSET + MESSAGE_FORMAT_LEN;
const SIGNERS_START: usize = SIGNER_COUNT_OFFSET + SIGNER_COUNT_LEN;

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
    /// Validates a Solana offchain message preamble
    fn validate(&self, actual_message_length: usize) -> Result<(), FatalError> {
        if self.signing_domain != *b"\xffsolana offchain" {
            return Err(FatalError::DeserializationFailed(
                "Invalid Solana signing domain in preamble".to_string(),
            ));
        }
        // 0 and 1 are the supported header versions
        // Version 1 added newline support to ASCII messages, and is otherwise identical
        if self.header_version != 0 && self.header_version != 1 {
            return Err(FatalError::DeserializationFailed(format!(
                    "Invalid header version in preamble: only versions 0 and 1 are supported, but version {} was provided", self.header_version
        )));
        }
        // Format 0 is the ASCII, hw-wallet compatible format
        if self.message_format != 0 {
            return Err(FatalError::DeserializationFailed(format!(
                    "Invalid message format in preamble: only format 0 is supported, but format {} was provided", self.message_format
        )));
        }
        if self.signer_count != 1 {
            return Err(FatalError::DeserializationFailed(format!(
                    "Invalid signer count in preamble: only a single signer is currently supported, but the count was {}", self.signer_count
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

#[derive(Debug)]
enum UnpackedSolanaMessage<S: Spec> {
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
    fn json_bytes(&self) -> &[u8] {
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

    fn chain_hash(&self) -> &[u8; 32] {
        match self {
            UnpackedSolanaMessage::V0 { chain_hash, .. }
            | UnpackedSolanaMessage::V1 { chain_hash, .. } => chain_hash,
        }
    }

    fn signed_bytes(&self) -> &[u8] {
        match self {
            UnpackedSolanaMessage::V0 { signed_bytes, .. }
            | UnpackedSolanaMessage::V1 { signed_bytes, .. } => signed_bytes,
        }
    }
}

fn charge_sig_gas<S: Spec>(
    signature: &<S::CryptoSpec as CryptoSpec>::Signature,
    msg_len: usize,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
    MeteredSignature::new::<S>(signature.clone())
        .charge_gas(meter, msg_len)
        .map_err(|e| match e {
            MeteredSigVerificationError::GasError(e) => {
                AuthenticationError::OutOfGas(format!("Signature verification ran out of gas: {e}"))
            }
            MeteredSigVerificationError::BadSignature(e) => AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(e.to_string()),
                raw_tx_hash,
            ),
        })
}

/// Verifies signatures with gas metering and caching, handling both single-sig and multisig.
fn verify_signatures<S: Spec>(
    unpacked: &UnpackedSolanaMessage<S>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<(), AuthenticationError> {
    let signed_bytes = unpacked.signed_bytes();

    // 1. Charge gas for all signatures before checking the cache
    match unpacked {
        UnpackedSolanaMessage::V0 { signature, .. } => {
            charge_sig_gas::<S>(signature, signed_bytes.len(), raw_tx_hash, meter)?;
        }
        UnpackedSolanaMessage::V1 { signatures, .. } => {
            for sig in signatures {
                charge_sig_gas::<S>(&sig.signature, signed_bytes.len(), raw_tx_hash, meter)?;
            }
        }
    }

    // 2. Check cache (native-only)
    #[cfg(feature = "native")]
    if let Some(known_result) = SIGNATURE_CACHE.get(&raw_tx_hash) {
        return known_result;
    }

    // 3. Verify signatures (unmetered)
    let res = match unpacked {
        UnpackedSolanaMessage::V0 {
            pub_key, signature, ..
        } => signature.verify(pub_key, signed_bytes).map_err(|err| {
            AuthenticationError::FatalError(
                FatalError::SigVerificationFailed(err.to_string()),
                raw_tx_hash,
            )
        }),
        UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            ..
        } => {
            let all_keys: Vec<_> = signatures
                .iter()
                .map(|s| s.pub_key.clone())
                .chain(unused_pub_keys.iter().cloned())
                .collect();
            Multisig::new(*min_signers, all_keys)
                .verify_signature(signed_bytes, signatures)
                .map_err(|err| {
                    AuthenticationError::FatalError(
                        FatalError::SigVerificationFailed(err.to_string()),
                        raw_tx_hash,
                    )
                })
        }
    };

    // 4. Cache result (native-only)
    #[cfg(feature = "native")]
    SIGNATURE_CACHE.insert(raw_tx_hash, res.clone());

    res
}

/// Builds authorization data for either single-sig or multisig transactions.
fn build_auth_data<S: Spec>(
    unpacked: &UnpackedSolanaMessage<S>,
    uniqueness: UniquenessData,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthorizationData<S>, AuthenticationError> {
    match unpacked {
        UnpackedSolanaMessage::V0 { pub_key, .. } => {
            let credential_id =
                sov_modules_api::metered_credential::<S, S::CryptoSpec>(pub_key, meter)
                    .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

            Ok(AuthorizationData {
                uniqueness,
                tx_hash: raw_tx_hash,
                credential_id,
                credentials: Credentials::new(pub_key.clone()),
                default_address: credential_id.into(),
            })
        }
        UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            ..
        } => {
            let num_keys = (signatures.len() + unused_pub_keys.len()) as u32;
            meter
                .charge_linear_gas(S::gas_to_charge_for_credential(), num_keys)
                .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

            let all_keys: Vec<_> = signatures
                .iter()
                .map(|s| s.pub_key.clone())
                .chain(unused_pub_keys.iter().cloned())
                .collect();
            let multisig = Multisig::new(*min_signers, all_keys);
            let credential_id = multisig.credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>();

            Ok(AuthorizationData {
                uniqueness,
                tx_hash: raw_tx_hash,
                credential_id,
                credentials: Credentials::new(multisig),
                default_address: credential_id.into(),
            })
        }
    }
}

/// Verifies that the multisig address committed to in the signed message matches the one derived
/// from the multisig parameters in the transaction envelope. This prevents credential
/// malleability, where signed bytes are reused in a different multisig envelope to derive a
/// different account.
fn verify_multisig_commitment<S: Spec>(
    signed_address: S::Address,
    envelope_signatures: &SafeVec<PubKeyAndSignature<S::CryptoSpec>, MAX_SIGNERS>,
    envelope_unused_pub_keys: &SafeVec<<S::CryptoSpec as CryptoSpec>::PublicKey, MAX_SIGNERS>,
    envelope_min_signers: u8,
    raw_tx_hash: TxHash,
) -> Result<(), AuthenticationError> {
    let all_keys: Vec<_> = envelope_signatures
        .iter()
        .map(|s| s.pub_key.clone())
        .chain(envelope_unused_pub_keys.iter().cloned())
        .collect();
    let envelope_address: S::Address = Multisig::new(envelope_min_signers, all_keys)
        .credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>()
        .into();

    if signed_address != envelope_address {
        return Err(AuthenticationError::FatalError(
            FatalError::SigVerificationFailed(format!(
                "Multisig address mismatch: signed message commits to \
                 {signed_address}, but envelope parameters derive {envelope_address}"
            )),
            raw_tx_hash,
        ));
    }

    Ok(())
}

fn unpack_solana_message<S: Spec>(raw_tx: &[u8]) -> Result<UnpackedSolanaMessage<S>, FatalError> {
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
    if raw_tx[BORSH_VEC_LEN_PREFIX] == SPEC_COMPLIANT_DISCRIMINATOR {
        // signer_count sits after the borsh Vec<u8> length prefix plus the preamble offset.
        const RAW_TX_SIGNER_COUNT_OFFSET: usize = BORSH_VEC_LEN_PREFIX + SIGNER_COUNT_OFFSET;
        if raw_tx.len() <= RAW_TX_SIGNER_COUNT_OFFSET {
            return Err(FatalError::DeserializationFailed(
                "Message too short to read signer count".to_string(),
            ));
        }
        if raw_tx[RAW_TX_SIGNER_COUNT_OFFSET] > 1 {
            unpack_spec_compliant_multisig_message(raw_tx)
        } else {
            unpack_spec_compliant_message(raw_tx)
        }
    } else if raw_tx[BORSH_VEC_LEN_PREFIX] == MULTISIG_SIMPLE_DISCRIMINATOR {
        unpack_multisig_simple_message(raw_tx)
    } else {
        unpack_simple_message(raw_tx)
    }
}

fn unpack_spec_compliant_message<S: Spec>(
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

    // Unwrap: we just checked the length is >= single_key_preamble_len, so this can't underflow
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

fn unpack_simple_message<S: Spec>(raw_tx: &[u8]) -> Result<UnpackedSolanaMessage<S>, FatalError> {
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

fn unpack_multisig_simple_message<S: Spec>(
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

    // Validate bitfield: no out-of-range bits set
    let valid_bits_mask = (1u32 << signer_count) - 1;
    if signer_bitfield & !valid_bits_mask != 0 {
        return Err(FatalError::DeserializationFailed(format!(
            "Signer bitfield has bits set beyond signer_count ({signer_count})"
        )));
    }

    // Validate bitfield popcount matches number of signatures
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

fn unpack_spec_compliant_multisig_message<S: Spec>(
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

/// Decode bytes as a Sovereign SDK transaction, returning the message and tx info.
pub fn decode_solana_json_tx<S, D>(raw_tx: &[u8]) -> Result<D::Decodable, FatalError>
where
    S: Spec,
    D: DispatchCall<Spec = S>,
    <D as DispatchCall>::Decodable: Serialize + DeserializeOwned,
{
    let unpacked_message = unpack_solana_message::<S>(raw_tx)?;
    let json_bytes = unpacked_message.json_bytes();
    let deser_err = |e: serde_json::Error| FatalError::DeserializationFailed(e.to_string());
    let unsigned_tx = match &unpacked_message {
        UnpackedSolanaMessage::V0 { .. } => {
            SolanaOffchainUnsignedTransactionV0::<D, S>::unmetered_deserialize(json_bytes)
                .map_err(deser_err)?
                .into_unsigned_tx()
        }
        UnpackedSolanaMessage::V1 { .. } => {
            SolanaOffchainUnsignedTransactionV1::<D, S>::unmetered_deserialize(json_bytes)
                .map_err(deser_err)?
                .into_unsigned_tx()
        }
    };
    Ok(unsigned_tx.call())
}

pub fn authenticate<Accessor, S, D>(
    raw_tx: &[u8],
    runtime_chain_hash: &[u8; 32],
    runtime_chain_name: &'static str,
    state: &mut Accessor,
) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError>
where
    Accessor: ProvableStateReader<sov_state::User, Spec = S>,
    S: Spec,
    D: DispatchCall<Spec = S>,
    <D as DispatchCall>::Decodable: Serialize + DeserializeOwned,
{
    let raw_tx_hash = calculate_hash_metered::<Accessor, S>(raw_tx, state)
        .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    let unpacked_message = unpack_solana_message::<S>(raw_tx)
        .map_err(|e| AuthenticationError::FatalError(e, raw_tx_hash))?;

    // Deserialize the JSON unsigned transaction.
    // V0 (single-sig) and V1 (multisig) use different structs; for V1, we also verify
    // that the credential_id committed to in the signed message matches the envelope.
    let json_slice = unpacked_message.json_bytes();
    charge_gas_to_deserialize_json(json_slice, state).map_err(|e| {
        AuthenticationError::OutOfGas(format!(
            "Transaction deserialization run out of gas: {e}, tx hash {raw_tx_hash}"
        ))
    })?;
    let deser_err = |e: serde_json::Error| {
        AuthenticationError::FatalError(
            FatalError::DeserializationFailed(e.to_string()),
            raw_tx_hash,
        )
    };
    let (provided_chain_name, unsigned_tx) = match &unpacked_message {
        UnpackedSolanaMessage::V0 { .. } => {
            let tx = SolanaOffchainUnsignedTransactionV0::<D, S>::unmetered_deserialize(json_slice)
                .map_err(deser_err)?;
            (tx.chain_name.to_string(), tx.into_unsigned_tx())
        }
        UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            ..
        } => {
            let tx = SolanaOffchainUnsignedTransactionV1::<D, S>::unmetered_deserialize(json_slice)
                .map_err(deser_err)?;
            verify_multisig_commitment::<S>(
                tx.multisig_id,
                signatures,
                unused_pub_keys,
                *min_signers,
                raw_tx_hash,
            )?;
            (tx.chain_name.to_string(), tx.into_unsigned_tx())
        }
    };

    // Validate chain hash, chain name, and chain ID (shared for all variants)
    if *unpacked_message.chain_hash() != *runtime_chain_hash {
        return Err(AuthenticationError::FatalError(
            FatalError::InvalidChainHash {
                expected: hex::encode(runtime_chain_hash),
                got: hex::encode(unpacked_message.chain_hash()),
            },
            raw_tx_hash,
        ));
    }

    if provided_chain_name != runtime_chain_name {
        return Err(AuthenticationError::FatalError(
            FatalError::InvalidChainName {
                expected: runtime_chain_name.to_string(),
                got: provided_chain_name,
            },
            raw_tx_hash,
        ));
    }

    verify_chain_id(&unsigned_tx.details, raw_tx_hash)?;

    // Verify signatures (branches internally for single-sig vs multisig)
    verify_signatures::<S>(&unpacked_message, raw_tx_hash, state)?;

    // Build authorization data (branches internally for single-sig vs multisig)
    let authorization_data = build_auth_data::<S>(
        &unpacked_message,
        unsigned_tx.uniqueness,
        raw_tx_hash,
        state,
    )?;

    let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
        raw_tx_hash,
        authenticated_tx: unsigned_tx.details.into(),
    };

    Ok((
        tx_and_raw_hash,
        authorization_data,
        unsigned_tx.runtime_call,
    ))
}

#[cfg(test)]
pub mod test {
    use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
    use sov_mock_zkvm::crypto::Ed25519Signature;
    use sov_modules_api::PrivateKey;
    use sov_test_utils::TestSpec;

    use super::*;
    use crate::utils::make_preamble_for_message;

    const TEST_CHAIN_HASH: [u8; 32] = [0u8; 32];

    #[test]
    fn test_unpack_with_preamble() {
        let message = b"{\"test\":\"abcd\"}";
        let message_len = message.len() as u16;

        let pubkey = Ed25519PrivateKey::generate().pub_key();
        let signature: Ed25519Signature = [4u8; 64].as_slice().try_into().unwrap();

        let preamble = make_preamble_for_message(pubkey.bytes(), &TEST_CHAIN_HASH, message_len);

        let mut signed_message = Vec::new();
        signed_message.extend_from_slice(&preamble);
        signed_message.extend_from_slice(message);

        let envelope = SolanaOffchainSpecCompliantMessage::<TestSpec> {
            signed_message_with_preamble: signed_message.clone(),
            signature: signature.clone(),
        };

        let serialized = borsh::to_vec(&envelope).unwrap();

        let unpacked = unpack_solana_message::<TestSpec>(&serialized).unwrap();
        let UnpackedSolanaMessage::V0 {
            pub_key: got_pk,
            signature: got_sig,
            chain_hash: got_ch,
            signed_bytes: got_sb,
            ..
        } = &unpacked
        else {
            panic!("Expected SingleSig variant");
        };
        assert_eq!(*got_pk, pubkey);
        assert_eq!(*got_sig, signature);
        assert_eq!(*got_ch, TEST_CHAIN_HASH);
        assert_eq!(unpacked.json_bytes(), message);
        assert_eq!(*got_sb, signed_message);
    }

    #[test]
    fn test_unpack_raw_message() {
        let message = b"{\"test\":\"abcd\"}";

        let pubkey = Ed25519PrivateKey::generate().pub_key();
        let signature: Ed25519Signature = [4u8; 64].as_slice().try_into().unwrap();

        let raw_message = SolanaOffchainSimpleMessage::<TestSpec> {
            signed_message: message.to_vec(),
            chain_hash: TEST_CHAIN_HASH,
            pubkey: pubkey.clone(),
            signature: signature.clone(),
        };

        let serialized = borsh::to_vec(&raw_message).unwrap();

        let unpacked = unpack_solana_message::<TestSpec>(&serialized).unwrap();
        let UnpackedSolanaMessage::V0 {
            pub_key: got_pk,
            signature: got_sig,
            chain_hash: got_ch,
            signed_bytes: got_sb,
            ..
        } = &unpacked
        else {
            panic!("Expected SingleSig variant");
        };
        assert_eq!(*got_pk, pubkey);
        assert_eq!(*got_sig, signature);
        assert_eq!(*got_ch, TEST_CHAIN_HASH);
        assert_eq!(unpacked.json_bytes(), message);
        assert_eq!(got_sb.as_slice(), message);
    }

    #[test]
    fn test_unpack_multisig_simple_message() {
        let json_message = b"{\"test\":\"abcd\"}";
        let mut wire_bytes = vec![MULTISIG_SIMPLE_DISCRIMINATOR];
        wire_bytes.extend_from_slice(json_message);

        let pubkey1 = Ed25519PrivateKey::generate().pub_key();
        let pubkey2 = Ed25519PrivateKey::generate().pub_key();
        let pubkey3 = Ed25519PrivateKey::generate().pub_key();
        let sig1: Ed25519Signature = [1u8; 64].as_slice().try_into().unwrap();
        let sig2: Ed25519Signature = [2u8; 64].as_slice().try_into().unwrap();

        let envelope = SolanaOffchainSimpleMultisigMessage::<TestSpec> {
            wire_bytes,
            chain_hash: TEST_CHAIN_HASH,
            signatures: vec![
                PubKeyAndSignature {
                    signature: sig1,
                    pub_key: pubkey1.clone(),
                },
                PubKeyAndSignature {
                    signature: sig2,
                    pub_key: pubkey2.clone(),
                },
            ]
            .try_into()
            .unwrap(),
            unused_pub_keys: vec![pubkey3.clone()].try_into().unwrap(),
            min_signers: 2,
        };

        let serialized = borsh::to_vec(&envelope).unwrap();

        let unpacked = unpack_solana_message::<TestSpec>(&serialized).unwrap();
        let UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            chain_hash,
            signed_bytes,
            json_start,
        } = &unpacked
        else {
            panic!("Expected Multisig variant");
        };
        assert_eq!(signatures.len(), 2);
        assert_eq!(signatures[0].pub_key, pubkey1);
        assert_eq!(signatures[1].pub_key, pubkey2);
        assert_eq!(unused_pub_keys.as_ref(), &[pubkey3]);
        assert_eq!(*min_signers, 2);
        assert_eq!(*chain_hash, TEST_CHAIN_HASH);
        assert_eq!(*json_start, 0);
        // signed_bytes should be the JSON only (discriminator stripped)
        assert_eq!(signed_bytes, json_message);
        assert_eq!(unpacked.json_bytes(), json_message);
    }

    #[test]
    fn test_invalid_preamble() {
        let message = b"{\"test\":\"abcd\"}";

        let pubkey = Ed25519PrivateKey::generate().pub_key();
        let signature: Ed25519Signature = [4u8; 64].as_slice().try_into().unwrap();

        // Create invalid preamble
        let mut header = Vec::<u8>::new();
        header.extend(b"\xffsolanaXoffchain"); // Wrong domain
        header.push(0);
        header.extend(TEST_CHAIN_HASH);
        header.push(0);
        header.push(1);
        header.extend(pubkey.bytes());
        header.extend(&(message.len() as u16).to_le_bytes());

        let mut signed_message = Vec::new();
        signed_message.extend_from_slice(&header);
        signed_message.extend_from_slice(message);

        let envelope = SolanaOffchainSpecCompliantMessage::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signature: signature.clone(),
        };

        let serialized = borsh::to_vec(&envelope).unwrap();

        let result = unpack_solana_message::<TestSpec>(&serialized);
        assert!(result.is_err());
        assert!(matches!(result, Err(FatalError::DeserializationFailed(_))));
    }

    #[test]
    fn test_unpack_spec_compliant_multisig_message() {
        use crate::utils::make_multisig_preamble_for_message;

        let json_message = b"{\"test\":\"abcd\"}";
        let message_len = json_message.len() as u16;

        let pubkey1 = Ed25519PrivateKey::generate().pub_key();
        let pubkey2 = Ed25519PrivateKey::generate().pub_key();
        let pubkey3 = Ed25519PrivateKey::generate().pub_key();
        let sig1: Ed25519Signature = [1u8; 64].as_slice().try_into().unwrap();
        let sig3: Ed25519Signature = [3u8; 64].as_slice().try_into().unwrap();

        let preamble = make_multisig_preamble_for_message(
            &[*pubkey1.bytes(), *pubkey2.bytes(), *pubkey3.bytes()],
            &TEST_CHAIN_HASH,
            message_len,
        );

        let mut signed_message = preamble.clone();
        signed_message.extend_from_slice(json_message);

        // Signers 0 and 2 signed (bitfield = 0b101 = 5), signer 1 did not
        let envelope = SolanaOffchainSpecCompliantMultisigMessage::<TestSpec> {
            signed_message_with_preamble: signed_message.clone(),
            signatures: vec![sig1.clone(), sig3.clone()].try_into().unwrap(),
            signer_bitfield: 0b101,
            min_signers: 2,
        };

        let serialized = borsh::to_vec(&envelope).unwrap();
        let unpacked = unpack_solana_message::<TestSpec>(&serialized).unwrap();

        let UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            chain_hash,
            signed_bytes,
            json_start,
        } = &unpacked
        else {
            panic!("Expected Multisig variant");
        };

        // Signatures should be paired with pubkeys 0 and 2 (matching bitfield order)
        assert_eq!(signatures.len(), 2);
        assert_eq!(signatures[0].pub_key, pubkey1);
        assert_eq!(signatures[0].signature, sig1);
        assert_eq!(signatures[1].pub_key, pubkey3);
        assert_eq!(signatures[1].signature, sig3);

        // Pubkey 1 is unused
        assert_eq!(unused_pub_keys.as_ref(), &[pubkey2]);

        assert_eq!(*min_signers, 2);
        assert_eq!(*chain_hash, TEST_CHAIN_HASH);
        assert_eq!(*json_start, preamble.len());
        assert_eq!(*signed_bytes, signed_message);
        assert_eq!(unpacked.json_bytes(), json_message);
    }

    #[test]
    fn test_spec_compliant_multisig_bitfield_popcount_mismatch() {
        use crate::utils::make_multisig_preamble_for_message;

        let json_message = b"{\"test\":\"abcd\"}";
        let pubkey1 = Ed25519PrivateKey::generate().pub_key();
        let pubkey2 = Ed25519PrivateKey::generate().pub_key();
        let sig1: Ed25519Signature = [1u8; 64].as_slice().try_into().unwrap();

        let preamble = make_multisig_preamble_for_message(
            &[*pubkey1.bytes(), *pubkey2.bytes()],
            &TEST_CHAIN_HASH,
            json_message.len() as u16,
        );

        let mut signed_message = preamble;
        signed_message.extend_from_slice(json_message);

        // 1 signature but bitfield says 2 signers (0b11)
        let envelope = SolanaOffchainSpecCompliantMultisigMessage::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signatures: vec![sig1].try_into().unwrap(),
            signer_bitfield: 0b11,
            min_signers: 2,
        };

        let serialized = borsh::to_vec(&envelope).unwrap();
        let result = unpack_solana_message::<TestSpec>(&serialized);
        let Err(FatalError::DeserializationFailed(err_msg)) = result else {
            panic!("Expected DeserializationFailed, got: {result:?}");
        };
        assert!(
            err_msg.contains("popcount"),
            "Expected popcount mismatch error, got: {err_msg}"
        );
    }

    #[test]
    fn test_spec_compliant_multisig_bitfield_out_of_range() {
        use crate::utils::make_multisig_preamble_for_message;

        let json_message = b"{\"test\":\"abcd\"}";
        let pubkey1 = Ed25519PrivateKey::generate().pub_key();
        let pubkey2 = Ed25519PrivateKey::generate().pub_key();
        let sig1: Ed25519Signature = [1u8; 64].as_slice().try_into().unwrap();

        let preamble = make_multisig_preamble_for_message(
            &[*pubkey1.bytes(), *pubkey2.bytes()],
            &TEST_CHAIN_HASH,
            json_message.len() as u16,
        );

        let mut signed_message = preamble;
        signed_message.extend_from_slice(json_message);

        // Bit 2 is set but there are only 2 signers (indices 0 and 1)
        let envelope = SolanaOffchainSpecCompliantMultisigMessage::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signatures: vec![sig1].try_into().unwrap(),
            signer_bitfield: 0b100,
            min_signers: 1,
        };

        let serialized = borsh::to_vec(&envelope).unwrap();
        let result = unpack_solana_message::<TestSpec>(&serialized);
        let Err(FatalError::DeserializationFailed(err_msg)) = result else {
            panic!("Expected DeserializationFailed, got: {result:?}");
        };
        assert!(
            err_msg.contains("bits set beyond"),
            "Expected out-of-range bitfield error, got: {err_msg}"
        );
    }
}
