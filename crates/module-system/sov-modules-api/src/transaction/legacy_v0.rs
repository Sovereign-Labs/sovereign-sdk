//! Transitional support for the pre-fork ("legacy") V0 transaction encoding.
//!
//! The multisig and accounts hard fork (#2892) changed the V0 wire format in ways that break
//! borsh-encoded transactions produced by pre-fork clients:
//!
//! 1. [`Version0`] gained a trailing `address_override: Option<S::Address>` field (#2770).
//! 2. The signing payload became the version-prefixed
//!    [`TransactionSigningPayload`](crate::transaction::TransactionSigningPayload), which also
//!    carries `address_override`, instead of `borsh(UnsignedTransaction) ++ chain_hash` (#2688,
//!    #2876).
//! 3. `TxDetails::chain_id` was repurposed as `chain_hash_fragment` (#3035).
//!
//! To let clients lag behind on the upgrade, the standard authenticator accepts both encodings. A
//! V0 envelope that ends right after `details` is decoded as a legacy transaction with
//! `address_override = None` ([`DecodedTransaction::LegacyV0`]) and authenticated with the
//! pre-fork rules: the details field must hold the rollup's `CHAIN_ID`, and the signature must
//! verify over the legacy payload ([`legacy_signing_bytes`]) for one of the chain hashes valid at
//! the execution height.
//!
//! Legacy decoding and authentication must be retained to replay historical transactions during
//! resync, even after all clients have upgraded. A future deactivation height should gate legacy
//! authentication using the rollup height being executed, preserving existing execution behavior
//! (including gas charges) below that height. Decoding must remain available for historical data.
//! No deactivation height is currently enforced.

use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::CryptoSpec;

use crate::capabilities::UniquenessData;
use crate::transaction::{Transaction, TransactionCallable, TxDetails, Version0, Version1};
use crate::{CryptoSpecExt, Spec};

/// A transaction decoded from its wire encoding, together with which encoding was used.
#[derive(derive_more::Debug)]
pub enum DecodedTransaction<
    R: TransactionCallable,
    S: Spec,
    C: CryptoSpecExt = <S as Spec>::CryptoSpec,
> {
    /// A transaction in the current encoding.
    Current(Transaction<R, S, C>),
    /// A V0 transaction in the pre-fork encoding, which has no `address_override` field. The
    /// decoded `address_override` is always `None`.
    LegacyV0(Version0<R, S, C>),
}

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> DecodedTransaction<R, S, C> {
    /// Returns the decoded transaction, discarding the encoding information.
    pub fn into_transaction(self) -> Transaction<R, S, C> {
        match self {
            Self::Current(tx) => tx,
            Self::LegacyV0(tx) => Transaction::V0(tx),
        }
    }
}

/// Decodes a [`Transaction`] from `reader`, accepting both the current and the legacy V0 encoding.
///
/// This consumes exactly the same bytes as the derived [`BorshDeserialize`] impl of
/// [`Transaction`], with one exception: a V0 envelope whose input ends right after `details` is
/// accepted and returned as [`DecodedTransaction::LegacyV0`] instead of failing with an
/// unexpected end of input.
pub fn deserialize_transaction_reader<R, S, C, Reader>(
    reader: &mut Reader,
) -> io::Result<DecodedTransaction<R, S, C>>
where
    R: TransactionCallable,
    S: Spec,
    C: CryptoSpecExt,
    <C as CryptoSpec>::Signature: BorshDeserialize,
    <C as CryptoSpec>::PublicKey: BorshDeserialize,
    Reader: io::Read,
{
    // Borsh encodes the `Transaction` discriminant as a single byte: `V0` = 0, `V1` = 1.
    match u8::deserialize_reader(reader)? {
        0 => {
            // The field order must match `Version0`.
            let signature =
                <<C as CryptoSpec>::Signature as BorshDeserialize>::deserialize_reader(reader)?;
            let pub_key =
                <<C as CryptoSpec>::PublicKey as BorshDeserialize>::deserialize_reader(reader)?;
            let runtime_call = <R::Call as BorshDeserialize>::deserialize_reader(reader)?;
            let uniqueness = UniquenessData::deserialize_reader(reader)?;
            let details = TxDetails::<S>::deserialize_reader(reader)?;

            let Some(option_tag) = read_byte_or_eof(reader)? else {
                return Ok(DecodedTransaction::LegacyV0(Version0 {
                    signature,
                    pub_key,
                    runtime_call,
                    uniqueness,
                    details,
                    address_override: None,
                }));
            };
            // Mirrors borsh's `Option` decoding; its first byte has already been consumed above.
            let address_override = match option_tag {
                0 => None,
                1 => Some(<S::Address as BorshDeserialize>::deserialize_reader(
                    reader,
                )?),
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Invalid Option representation: {other}. The first byte must be 0 or 1"
                        ),
                    ))
                }
            };
            Ok(DecodedTransaction::Current(Transaction::V0(Version0 {
                signature,
                pub_key,
                runtime_call,
                uniqueness,
                details,
                address_override,
            })))
        }
        1 => Ok(DecodedTransaction::Current(Transaction::V1(
            Version1::<R, S, C>::deserialize_reader(reader)?,
        ))),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Unexpected variant tag: {other}"),
        )),
    }
}

/// Reads one byte from `reader`, returning `None` if the input is exhausted.
fn read_byte_or_eof(reader: &mut impl io::Read) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(byte[0])),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

/// Serializes the pre-fork signing payload of `tx` for `chain_hash`:
/// `borsh(runtime_call) ++ borsh(uniqueness) ++ borsh(details) ++ chain_hash`.
///
/// Before the fork this was `borsh(UnsignedTransaction) ++ chain_hash`, with `UnsignedTransaction`
/// consisting of exactly those three fields. The authenticator only verifies transactions decoded
/// as [`DecodedTransaction::LegacyV0`] against this payload.
pub fn legacy_signing_bytes<R: TransactionCallable, S: Spec, C: CryptoSpecExt>(
    tx: &Transaction<R, S, C>,
    chain_hash: &[u8; 32],
) -> Vec<u8> {
    let (runtime_call, uniqueness, details) = match tx {
        Transaction::V0(tx) => (&tx.runtime_call, &tx.uniqueness, &tx.details),
        Transaction::V1(tx) => (&tx.runtime_call, &tx.uniqueness, &tx.details),
    };
    let mut bytes = Vec::new();
    runtime_call
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    uniqueness
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    details
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    bytes.extend_from_slice(chain_hash);
    bytes
}

/// Serializes `tx` in the pre-fork V0 envelope, i.e. the current [`Transaction::V0`] encoding
/// without the trailing `address_override` field.
///
/// This is the inverse of the [`DecodedTransaction::LegacyV0`] branch of
/// [`deserialize_transaction_reader`] and exists for tests and tooling that need to produce
/// legacy transactions; the runtime never emits this encoding.
///
/// # Panics
/// Panics if `tx.address_override` is set, since the legacy envelope cannot represent it.
pub fn legacy_v0_envelope_bytes<R, S, C>(tx: &Version0<R, S, C>) -> Vec<u8>
where
    R: TransactionCallable,
    S: Spec,
    C: CryptoSpecExt,
    <C as CryptoSpec>::Signature: BorshSerialize,
    <C as CryptoSpec>::PublicKey: BorshSerialize,
{
    assert!(
        tx.address_override.is_none(),
        "the legacy V0 envelope cannot carry an address override"
    );
    let mut bytes = vec![0u8]; // `Transaction::V0` discriminant.
    tx.signature
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    tx.pub_key
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    tx.runtime_call
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    tx.uniqueness
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    tx.details
        .serialize(&mut bytes)
        .expect("Serialization to vec is infallible");
    bytes
}
