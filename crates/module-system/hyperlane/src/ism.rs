//! Implementations of Interchain Security Modules (ISM)

use std::collections::HashSet;

use anyhow::{Context as _, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, GasMeter, HexHash, HexString, SafeVec, Spec};

use crate::crypto::{
    compute_hash_for_signatures, decode_signature, ec_recover, eth_address_from_public_key,
    RecoverableSignature,
};
use crate::types::Message;
use crate::{EthAddress, HyperlaneAddress};

const MAX_VALIDATORS: usize = 128;
// See <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/isms/libs/MessageIdMultisigIsmMetadata.sol#L9>
const ADDRESS_SIZE: usize = 32;
const MERKLE_ROOT_SIZE: usize = 32;
const MERKLE_INDEX_SIZE: usize = 4;
const SIGNATURE_SIZE: usize = 65;
const MERKLE_INDEX_OFFSET: usize = ADDRESS_SIZE + MERKLE_ROOT_SIZE;
const SIGNATURES_OFFSET: usize = MERKLE_INDEX_OFFSET + MERKLE_INDEX_SIZE;

/// Represents the available ISMs
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    JsonSchema,
    UniversalWallet,
)]
pub enum Ism {
    /// Performs no validation. Will accept any message - useful for testing
    AlwaysTrust,
    /// Accepts all messages from a trusted relayer
    TrustedRelayer {
        /// The address of the trusted relayer, in [`HyperlaneAddress`] format
        relayer: HexHash,
    },
    /// Accepts messages if signed by `threshold` or more of the provided `validators`
    MessageIdMultisig {
        /// The addresses of the validators
        validators: SafeVec<EthAddress, MAX_VALIDATORS>,
        /// The number of signatures required to accept a message
        threshold: u32,
    },
}

/// Represents the available ISM types.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
)]
#[repr(u8)]
#[borsh(use_discriminant = false)]
pub enum IsmKind {
    Unused = 0,
    Routing = 1,
    Aggregation = 2,
    LegacyMultisig = 3,
    MerkleRootMultisig = 4,
    MessageIdMultisig = 5,
    Null = 6, // used with relayer carrying no metadata
    CcipRead = 7,
}

impl Ism {
    /// Verify that a message is valid for the given ISM
    pub fn verify<S: Spec>(
        &self,
        context: &Context<S>,
        message: &Message,
        metadata: &HexString,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<()>
    where
        S::Address: HyperlaneAddress,
    {
        match self {
            Ism::AlwaysTrust => Ok(()),
            Ism::TrustedRelayer { relayer } => {
                anyhow::ensure!(
                    &context.sender().to_sender() == relayer,
                    "Only {} is trusted to relay messages for this ISM",
                    relayer,
                );
                Ok(())
            }
            Ism::MessageIdMultisig {
                validators: original_validators,
                threshold,
            } => {
                let metadata = MessageIdMultisigIsmMetadata::decode(&metadata.0, *threshold)?;

                let hash = compute_hash_for_signatures(
                    message,
                    &metadata.origin_merkle_tree,
                    &metadata.merkle_root,
                    metadata.merkle_index,
                    gas_meter,
                )?;
                let mut validators: HashSet<&EthAddress> = original_validators.iter().collect();
                // We've already checked that exactly the right number of signatures are present,
                // so we can just iterate over them and check that each one is valid and in the set, removing the validator from the set as we go.
                for signature in metadata.signatures {
                    let pubkey_bytes =
                        ec_recover(hash.0, &signature, gas_meter).context("Invalid signature")?;
                    let addr = eth_address_from_public_key(pubkey_bytes, gas_meter)?;
                    if !validators.remove(&addr) {
                        // We make the error message more helpful by checking if the validator was in the original list of validators.
                        // This lets us distinguish between double-signing and unknown validators.
                        if original_validators.contains(&addr) {
                            return Err(anyhow::anyhow!(
                                "Not enough unique validators signed the message. {} signed multiple times.", addr
                            ));
                        } else {
                            return Err(anyhow::anyhow!(
                                "Not enough unique validators signed the message. {} is not an allowed validator.", addr
                            ));
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Get the ISM type for the given ISM
    pub fn ism_kind(&self) -> IsmKind {
        match self {
            Ism::AlwaysTrust | // AlwaysTrust is used with relayer carrying no metadata
            Ism::TrustedRelayer { .. } => IsmKind::Null, // TrustedRelayer is used with relayer carrying no metadata
            Ism::MessageIdMultisig { .. } => IsmKind::MessageIdMultisig,
        }
    }
}

/// Metadata for Message ID Multisig ISM
/// See <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/isms/libs/MessageIdMultisigIsmMetadata.sol#L4-L5>
struct MessageIdMultisigIsmMetadata {
    origin_merkle_tree: HexHash,
    merkle_root: HexHash,
    merkle_index: u32,
    signatures: Vec<RecoverableSignature>,
}

impl MessageIdMultisigIsmMetadata {
    /// Decode the metadata from a message.
    // See <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/isms/libs/MessageIdMultisigIsmMetadata.sol#L9>
    // for the reference on the expected format.
    fn decode(metadata: &[u8], num_signatures: u32) -> Result<Self> {
        let num_signatures: usize = num_signatures.try_into().expect("Threshold is too big for a usize. This is only possible on 16-bit platforms, which are not supported.");

        let Some(expected_len) = expected_len_check_overflow_u32(num_signatures) else {
            anyhow::bail!(
                "Invalid metadata length, calulation overflowed {}",
                num_signatures
            );
        };

        anyhow::ensure!(
            metadata.len() == expected_len,
            "Invalid metadata length: expected {}, got {}",
            expected_len,
            metadata.len()
        );
        // Safety: we've already checked the length so all unwraps are infallible
        let origin_merkle_tree: HexHash = HexString(metadata[0..ADDRESS_SIZE].try_into().unwrap());
        let merkle_root: HexHash = HexString(
            metadata[ADDRESS_SIZE..MERKLE_INDEX_OFFSET]
                .try_into()
                .unwrap(),
        );
        let merkle_index = u32::from_be_bytes(
            metadata[MERKLE_INDEX_OFFSET..SIGNATURES_OFFSET]
                .try_into()
                .unwrap(),
        );
        let signatures = metadata[SIGNATURES_OFFSET..]
            .chunks_exact(SIGNATURE_SIZE)
            .map(decode_signature)
            .collect::<Result<Vec<_>>>()?;

        // Sanity check that the number of signatures is correct. We already checked the length at the top of this function,
        // so a failure here indicates a bug.
        assert!(
            signatures.len() == num_signatures,
            "Invalid number of signatures: expected {}, got {}",
            num_signatures,
            signatures.len()
        );

        Ok(Self {
            origin_merkle_tree,
            merkle_root,
            merkle_index,
            signatures,
        })
    }
}

fn expected_len_check_overflow_u32(num_signatures: usize) -> Option<usize> {
    let total_sig_size: usize = num_signatures.checked_mul(SIGNATURE_SIZE)?;
    let expected_len = ADDRESS_SIZE
        .checked_add(MERKLE_ROOT_SIZE)?
        .checked_add(MERKLE_INDEX_SIZE)?
        .checked_add(total_sig_size)?;

    // We don’t support platforms where `usize` is smaller than `u32`.
    // If `expected_len` overflows, we’ll catch it during native execution —
    // where `usize` is most likely a `u64`.
    TryInto::<u32>::try_into(expected_len).ok()?;
    Some(expected_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overflow_foo() {
        let len_res = expected_len_check_overflow_u32(100);
        assert!(len_res.is_some());

        let len_res = expected_len_check_overflow_u32(u32::MAX as usize);
        assert!(len_res.is_none());
    }
}
