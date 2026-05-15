mod parsing;
mod payload;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::capabilities::AuthorizationData;
use sov_modules_api::capabilities::{
    calculate_hash_metered, calculate_non_malleable_hash_metered, verify_chain_id,
    AuthenticationError, AuthenticationOutput, FatalError, ReplayHashMaterial, UniquenessData,
};
use sov_modules_api::transaction::AuthenticatedTransactionAndRawHash;
use sov_modules_api::transaction::Credentials;
use sov_modules_api::transaction::{v1::MAX_SIGNERS, PubKeyAndSignature};
use sov_modules_api::SafeVec;
use sov_modules_api::{
    charge_gas_to_deserialize_json, CryptoSpec, DispatchCall, GasMeter, GasSpec,
    MeteredSigVerificationError, MeteredSignature, Multisig, ProvableStateReader, Signature, Spec,
    TxHash,
};

#[cfg(feature = "native")]
use sov_modules_api::capabilities::{SignatureVerificationCache, DEFAULT_SIGNATURE_CACHE_SIZE};

use self::parsing::{unpack_solana_message, UnpackedSolanaMessage};

#[cfg(feature = "native")]
static SIGNATURE_CACHE: std::sync::LazyLock<SignatureVerificationCache<()>> =
    std::sync::LazyLock::new(|| SignatureVerificationCache::new(DEFAULT_SIGNATURE_CACHE_SIZE));

pub use self::parsing::simple::MULTISIG_SIMPLE_DISCRIMINATOR;
pub use self::parsing::simple::{
    SolanaOffchainSimpleEnvelope, SolanaOffchainSimpleMultisigEnvelope,
};
pub(crate) use self::parsing::spec_compliant::format_constants as spec_compliant_constants;
pub use self::parsing::spec_compliant::SPEC_COMPLIANT_DISCRIMINATOR;
pub use self::parsing::spec_compliant::{
    RawSolanaOffchainMessagePreamble, SolanaOffchainSpecCompliantEnvelope,
    SolanaOffchainSpecCompliantMultisigEnvelope,
};
pub use self::payload::{SolanaOffchainSigningPayloadV0, SolanaOffchainSigningPayloadV1};

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
    address_override: Option<S::Address>,
    raw_tx_hash: TxHash,
    meter: &mut impl GasMeter<Spec = S>,
) -> Result<AuthorizationData<S>, AuthenticationError> {
    let non_malleable_hash = calculate_non_malleable_hash_metered::<_, S>(
        match unpacked {
            UnpackedSolanaMessage::V0 { .. } => {
                ReplayHashMaterial::AlreadyNonMalleableHash(raw_tx_hash)
            }
            UnpackedSolanaMessage::V1 { .. } => {
                ReplayHashMaterial::VerifiedSignatureMessage(unpacked.signed_bytes())
            }
        },
        meter,
    )
    .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

    match unpacked {
        UnpackedSolanaMessage::V0 { pub_key, .. } => {
            let credential_id =
                sov_modules_api::metered_credential::<S, S::CryptoSpec>(pub_key, meter)
                    .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

            Ok(AuthorizationData {
                uniqueness,
                tx_hash: raw_tx_hash,
                non_malleable_hash,
                credential_id,
                credentials: Credentials::new(pub_key.clone()),
                default_address: credential_id.into(),
                address_override,
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
                non_malleable_hash,
                credential_id,
                credentials: Credentials::new(multisig),
                default_address: credential_id.into(),
                address_override,
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
            SolanaOffchainSigningPayloadV0::<D, S>::unmetered_deserialize(json_bytes)
                .map_err(deser_err)?
                .into_unsigned_transaction()
        }
        UnpackedSolanaMessage::V1 { .. } => {
            SolanaOffchainSigningPayloadV1::<D, S>::unmetered_deserialize(json_bytes)
                .map_err(deser_err)?
                .into_unsigned_transaction()
        }
    };
    Ok(unsigned_tx.runtime_call().clone())
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

    // Deserialize the Solana JSON signing payload.
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
    let (provided_chain_name, address_override, unsigned_tx) = match &unpacked_message {
        UnpackedSolanaMessage::V0 { .. } => {
            let tx = SolanaOffchainSigningPayloadV0::<D, S>::unmetered_deserialize(json_slice)
                .map_err(deser_err)?;
            let address_override = tx.address_override;
            (
                tx.chain_name.to_string(),
                address_override,
                tx.into_unsigned_transaction(),
            )
        }
        UnpackedSolanaMessage::V1 {
            signatures,
            unused_pub_keys,
            min_signers,
            ..
        } => {
            let tx = SolanaOffchainSigningPayloadV1::<D, S>::unmetered_deserialize(json_slice)
                .map_err(deser_err)?;
            verify_multisig_commitment::<S>(
                tx.multisig_id,
                signatures,
                unused_pub_keys,
                *min_signers,
                raw_tx_hash,
            )?;
            let address_override = tx.address_override;
            (
                tx.chain_name.to_string(),
                address_override,
                tx.into_unsigned_transaction(),
            )
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

    verify_chain_id(unsigned_tx.details(), raw_tx_hash)?;

    // Verify signatures (branches internally for single-sig vs multisig)
    verify_signatures::<S>(&unpacked_message, raw_tx_hash, state)?;

    // Build authorization data (branches internally for single-sig vs multisig)
    let authorization_data = build_auth_data::<S>(
        &unpacked_message,
        unsigned_tx.uniqueness(),
        address_override,
        raw_tx_hash,
        state,
    )?;

    let tx_and_raw_hash = AuthenticatedTransactionAndRawHash {
        raw_tx_hash,
        authenticated_tx: unsigned_tx.details().clone().into(),
    };

    Ok((
        tx_and_raw_hash,
        authorization_data,
        unsigned_tx.runtime_call().clone(),
    ))
}

#[cfg(test)]
pub mod test {
    use borsh::to_vec;
    use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
    use sov_mock_zkvm::crypto::Ed25519Signature;
    use sov_modules_api::PrivateKey;
    use sov_test_utils::TestSpec;

    use super::parsing::{unpack_solana_message, UnpackedSolanaMessage};
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

        let envelope = SolanaOffchainSpecCompliantEnvelope::<TestSpec> {
            signed_message_with_preamble: signed_message.clone(),
            signature: signature.clone(),
        };

        let serialized = to_vec(&envelope).unwrap();

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

        let raw_message = SolanaOffchainSimpleEnvelope::<TestSpec> {
            signed_message: message.to_vec(),
            chain_hash: TEST_CHAIN_HASH,
            pubkey: pubkey.clone(),
            signature: signature.clone(),
        };

        let serialized = to_vec(&raw_message).unwrap();

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

        let envelope = SolanaOffchainSimpleMultisigEnvelope::<TestSpec> {
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

        let serialized = to_vec(&envelope).unwrap();

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

        let envelope = SolanaOffchainSpecCompliantEnvelope::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signature: signature.clone(),
        };

        let serialized = to_vec(&envelope).unwrap();

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
        let envelope = SolanaOffchainSpecCompliantMultisigEnvelope::<TestSpec> {
            signed_message_with_preamble: signed_message.clone(),
            signatures: vec![sig1.clone(), sig3.clone()].try_into().unwrap(),
            signer_bitfield: 0b101,
            min_signers: 2,
        };

        let serialized = to_vec(&envelope).unwrap();
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
        let envelope = SolanaOffchainSpecCompliantMultisigEnvelope::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signatures: vec![sig1].try_into().unwrap(),
            signer_bitfield: 0b11,
            min_signers: 2,
        };

        let serialized = to_vec(&envelope).unwrap();
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
        let envelope = SolanaOffchainSpecCompliantMultisigEnvelope::<TestSpec> {
            signed_message_with_preamble: signed_message,
            signatures: vec![sig1].try_into().unwrap(),
            signer_bitfield: 0b100,
            min_signers: 1,
        };

        let serialized = to_vec(&envelope).unwrap();
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
