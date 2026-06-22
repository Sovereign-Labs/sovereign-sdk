mod error;

use std::ops::Range;
use std::sync::OnceLock;

use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::namespace_data::NamespaceData;
/// Reexport the [`Namespace`] from `celestia-types`
pub use celestia_types::nmt::Namespace;
pub use error::*;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHashTrait, CountedBufReader};
use sov_universal_wallet::schema::OverrideSchema;
use sov_universal_wallet::UniversalWallet;

use crate::envelope::{classify_and_decode, EnvelopeState};
use crate::shares::BlobIterator;
use crate::verifier::address::CelestiaAddress;
use crate::CelestiaHeader;

pub(crate) const SUPPORTED_SHARE_VERSION: u8 = 1;

#[derive(Debug, PartialEq, PartialOrd, Ord, Clone, Eq, Hash, Serialize, Deserialize)]
pub struct TmHash(pub tendermint::Hash);

// Schema type for TmHash to enable UniversalWallet support
#[derive(UniversalWallet)]
#[allow(dead_code)]
#[doc(hidden)]
pub struct TmHashSchema(#[sov_wallet(display(hex))] [u8; 32]);

impl OverrideSchema for TmHash {
    type Output = TmHashSchema;
}

impl BorshSerialize for TmHash {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        BorshSerialize::serialize(self.inner(), writer)
    }
}

impl BorshDeserialize for TmHash {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, std::io::Error> {
        let bytes = <[u8; 32] as BorshDeserialize>::deserialize(buf)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }

    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }
}

impl AsRef<[u8]> for TmHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl core::fmt::Display for TmHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", self.0)
    }
}

impl core::str::FromStr for TmHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let stripped = s.strip_prefix("0x").unwrap_or(s);
        let inner = tendermint::Hash::from_str(stripped)?;
        Ok(TmHash(inner))
    }
}

impl TmHash {
    pub fn inner(&self) -> &[u8; 32] {
        match self.0 {
            tendermint::Hash::Sha256(ref h) => h,
            // `Hash::None` is normalized at a higher layer (genesis predecessor placeholder),
            // so `TmHash` should never observe it.
            tendermint::Hash::None => unreachable!("Only the genesis block has a None hash, and we use a placeholder in that corner case")
        }
    }
}

impl BlockHashTrait for TmHash {}

impl From<TmHash> for [u8; 32] {
    fn from(val: TmHash) -> Self {
        *val.inner()
    }
}

impl From<[u8; 32]> for TmHash {
    fn from(value: [u8; 32]) -> Self {
        Self(tendermint::Hash::Sha256(value))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobWithSender {
    pub(crate) blob: CountedBufReader<BlobIterator>,
    // Range in the entire namespace
    pub(crate) range_in_namespace: Range<usize>,
    pub(crate) sender: CelestiaAddress,
    pub hash: HexHash,

    /// Interior, serde-skipped cache of the blob's envelope classification.
    ///
    /// Derived only from the authenticated DA accumulator
    /// ([`Self::compressed_verified_data`]); never a serialized witness claim.
    /// Empty after construction and after every (de)serialization, then
    /// recomputed deterministically. Ignored by `PartialEq`.
    ///
    /// Filled lazily by the `&self` accessors via `get_or_init`; native `advance`
    /// (which holds `&mut self`) extends it in place as it authenticates more chunks.
    /// The guest reconstructs it identically.
    ///
    /// Why `OnceLock` and not another cell:
    /// * The `&self` accessors (`verified_data`/`total_len`/`rollup_decode_failed`) —
    ///   the only blob read path compiled into the zk guest — must decode once and
    ///   return a `&[u8]` into owned storage, which requires interior mutability under
    ///   `&self`.
    /// * [`BlobReaderTrait`] is `Send + Sync`, so the cell must be `Sync`. That rules
    ///   out `RefCell`/`Cell`; `Mutex`/`RwLock` are `Sync` but add runtime locking and
    ///   poisoning for a value written at most once — overkill. `OnceLock::get_or_init`
    ///   is exactly "compute once under `&self`, hand out a reference."
    /// * Decoding stays lazy (first access), so a blob that is only size-gated decodes
    ///   nothing — preserving the partial-read / DoS bound.
    #[serde(skip)]
    envelope_state: OnceLock<EnvelopeState>,
}

impl PartialEq for BlobWithSender {
    fn eq(&self, other: &Self) -> bool {
        // The envelope cache is derived state, not identity: two blobs with the
        // same authenticated bytes are equal regardless of whether either has
        // lazily computed its classification yet.
        self.blob == other.blob
            && self.range_in_namespace == other.range_in_namespace
            && self.sender == other.sender
            && self.hash == other.hash
    }
}

/// Celestia-private accessors distinguishing the two byte streams a blob represents:
///
/// * **DA ("compressed")**: the payload bytes as actually posted to Celestia.
///   Share-occupancy math (inclusion proofs, namespace continuity) is defined over
///   these bytes and only these bytes.
/// * **Rollup**: the payload bytes exposed to the rollup via [`BlobReaderTrait`].
///
/// The two streams diverge when a blob is posted as a compressed envelope (they
/// coincide for legacy raw blobs); proof generation and verification must keep using
/// the `compressed_*` accessors so the share math stays tied to what is actually on DA.
impl BlobWithSender {
    /// DA payload bytes consumed so far. These are the bytes that
    /// inclusion proofs must cover.
    pub(crate) fn compressed_verified_data(&self) -> &[u8] {
        self.blob.accumulator()
    }

    /// Total DA payload length. Must always equal the `sequence_length`
    /// recorded in the blob's first share; the verifier enforces this.
    pub(crate) fn compressed_total_len(&self) -> usize {
        self.blob.total_len()
    }

    /// Envelope classification of the authenticated DA prefix, decoded
    /// once and cached. Derived purely from [`Self::compressed_verified_data`], so
    /// the guest recomputes it identically after the serde-skipped cache is dropped.
    fn envelope_state(&self) -> &EnvelopeState {
        self.envelope_state
            .get_or_init(|| classify_and_decode(self.compressed_verified_data()))
    }

    /// Rollup payload bytes observed by the rollup so far: the decoded bytes for a
    /// valid envelope, or the DA bytes for a legacy/malformed blob.
    pub(crate) fn rollup_verified_data(&self) -> &[u8] {
        match self.envelope_state() {
            EnvelopeState::Envelope(decoded) => &decoded.rollup,
            EnvelopeState::Legacy | EnvelopeState::Malformed => self.compressed_verified_data(),
        }
    }

    /// Total length of the rollup payload exposed to the rollup. For an envelope
    /// this is the authenticated header `rollup_len` (never the decodable length, so
    /// the STF completeness assert keeps its teeth); otherwise the DA length.
    pub(crate) fn rollup_total_len(&self) -> usize {
        match self.envelope_state() {
            EnvelopeState::Envelope(decoded) => decoded.header.rollup_len as usize,
            EnvelopeState::Legacy | EnvelopeState::Malformed => self.compressed_total_len(),
        }
    }
}

impl BlobReaderTrait for BlobWithSender {
    type Address = CelestiaAddress;
    type BlobHash = TmHash;

    fn sender(&self) -> CelestiaAddress {
        self.sender
    }

    fn hash(&self) -> Self::BlobHash {
        TmHash(tendermint::Hash::Sha256(self.hash.0))
    }

    fn verified_data(&self) -> &[u8] {
        self.rollup_verified_data()
    }

    fn total_len(&self) -> usize {
        self.rollup_total_len()
    }

    /// True iff the fully-provided authenticated DA bytes do not decode into a
    /// clean, exact, complete rollup payload (the slash signal). Derived only
    /// from authenticated bytes, so it is identical in native and zk execution.
    ///
    /// Lives on [`BlobReaderTrait`] (not a celestia-only method) because the generic
    /// `sov-blob-storage` accept path consults it: it cannot otherwise tell a corrupt
    /// sender (slash) from a withholding prover (fail closed).
    fn rollup_decode_failed(&self) -> bool {
        let EnvelopeState::Envelope(decoded) = self.envelope_state() else {
            // Legacy / malformed-as-raw: no decode layer, keep default behavior.
            return false;
        };
        if !decoded.clean {
            // Structurally invalid chunk (cap violation, overrun, LZ4 failure).
            return true;
        }
        if decoded.rollup.len() == decoded.header.rollup_len as usize {
            // All declared rollup bytes decoded: canonical only if the chunks
            // consumed the entire authenticated payload. A trailing DA tail
            // (`consumed < compressed_total_len`) is non-canonical and is flagged
            // here WITHOUT reading the tail.
            decoded.consumed != self.compressed_total_len()
        } else {
            // Fewer rollup bytes than declared: a sender fault (too few / truncated
            // chunks) only once the whole DA payload is present. A genuine partial
            // read (more DA bytes still available) is the prover-withholding case,
            // which the blob-storage completeness assert fails closed.
            self.compressed_verified_data().len() == self.compressed_total_len()
        }
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        // Ensure the cache is populated (computed once), then take a `&mut` to extend it
        // in place. `get_mut` borrows only the field — unlike `classification`, a `&self`
        // method whose borrow would cover all of `self` and block mutating `self.blob`
        // below.
        let _ = self
            .envelope_state
            .get_or_init(|| classify_and_decode(self.blob.accumulator()));
        match self.envelope_state.get_mut() {
            Some(EnvelopeState::Envelope(decoded)) => {
                let codec = decoded.header.codec;
                let rollup_len = decoded.header.rollup_len as usize;

                // Read at least `num_bytes` more rollup bytes (capped at the blob's
                // declared rollup length).
                let target = decoded
                    .rollup
                    .len()
                    .saturating_add(num_bytes)
                    .min(rollup_len);
                let mut covered = decoded.rollup.len();
                // Authenticate forward one whole chunk at a time — read each chunk's
                // 4-byte framing, validate it, then pull its payload into the accumulator
                // — until `covered` reaches `target` (or EOF / bad framing). The
                // accumulator always ends on a chunk boundary. No decoding here.
                while covered < target {
                    let pos = self.blob.accumulator().len();
                    if pos >= self.blob.total_len() {
                        break; // DA EOF
                    }
                    self.blob.advance(crate::envelope::CHUNK_HEADER_LEN);
                    let acc = self.blob.accumulator();
                    if acc.len() < pos + crate::envelope::CHUNK_HEADER_LEN {
                        break; // truncated framing at EOF
                    }
                    let chunk_rollup = u16::from_le_bytes([acc[pos], acc[pos + 1]]);
                    let chunk_encoded = u16::from_le_bytes([acc[pos + 2], acc[pos + 3]]);
                    // Validate framing BEFORE authenticating the (attacker-controlled-
                    // length) payload, so a 1-byte rollup read cannot be made to
                    // authenticate a u16-sized payload.
                    if !crate::envelope::chunk_framing_valid(
                        codec,
                        chunk_rollup,
                        chunk_encoded,
                        covered,
                        rollup_len,
                    ) {
                        // Bad framing: the 4 header bytes just advanced stay in the
                        // (authenticated) accumulator past `decoded.consumed`. Benign — the
                        // decoder below stops at the same boundary, and the guest's wholesale
                        // decode sees the identical accumulator, so both mark the blob unclean.
                        // Slightly wasteful, never a native/guest divergence.
                        break;
                    }
                    self.blob.advance(chunk_encoded as usize);
                    if self.blob.accumulator().len()
                        < pos + crate::envelope::CHUNK_HEADER_LEN + chunk_encoded as usize
                    {
                        break; // truncated payload at EOF
                    }
                    covered = covered.saturating_add(chunk_rollup as usize);
                }

                // Decode ONLY the chunks appended since the last decode, in place into
                // `decoded.rollup` (which already holds the prefix). `decoded.consumed` is the
                // chunk-boundary offset where the previous decode stopped, so `[consumed..]` is
                // exactly the new chunks (a partial trailing chunk is left unconsumed). Decoding
                // into the prefix buffer is required by the linked codec — its rolling dictionary
                // is the tail of the decoded output — and makes concatenated incremental decodes
                // equal a wholesale `classify_and_decode` byte-for-byte, so native and guest stay
                // in lockstep. O(bytes pulled) per call, not O(total).
                let consumed = decoded.consumed;
                let acc = self.blob.accumulator();
                let (new_consumed, new_clean) = crate::envelope::decode_chunks(
                    &mut decoded.rollup,
                    &acc[consumed..],
                    codec,
                    rollup_len,
                );
                decoded.consumed += new_consumed;
                decoded.clean &= new_clean;
            }
            // Legacy / malformed-as-raw: rollup and DA bytes coincide.
            _ => {
                self.blob.advance(num_bytes);
            }
        }
        self.verified_data()
    }
}

/// Data that is required for extracting the relevant blobs from the namespace
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub(crate) struct NamespaceRelevantData {
    /// Celestia namespace.
    pub(crate) namespace: Namespace,
    /// All relevant rollup shares as they appear in an extended data square, with proofs.
    pub(crate) data: NamespaceData,
}

impl NamespaceRelevantData {
    #[cfg(feature = "native")]
    pub(crate) fn new(namespace: Namespace, data: NamespaceData) -> Self {
        #[cfg(debug_assertions)]
        {
            for row in data.rows() {
                for share in &row.shares {
                    assert_eq!(
                        share.namespace(),
                        namespace,
                        "Share from different namespace {} (expected {})",
                        String::from_utf8_lossy(share.namespace().as_bytes()),
                        String::from_utf8_lossy(namespace.as_bytes())
                    );
                }
            }
        }
        Self { namespace, data }
    }

    #[cfg(feature = "native")]
    pub(crate) fn get_blobs_with_sender(&self) -> Vec<BlobWithSender> {
        let mut output = Vec::new();
        let ns_iterator = crate::shares::NamespaceDataIterator::new(&self.data);

        for share_seq in ns_iterator {
            #[cfg(debug_assertions)]
            {
                share_seq.check_consistency();
            }
            // Commitment
            let commitment =
                celestia_types::Commitment::from_shares(self.namespace, &share_seq.shares)
                    .expect("blob must be valid");
            let hash = HexHash::new(*commitment.hash());

            let range_in_namespace = share_seq.range_in_ns.clone();
            let Ok(blob) = crate::shares::Blob::try_from(share_seq) else {
                tracing::warn!("Failed to create blob from share sequence. Only can happen if support for share version above 1 is not added yet.");
                continue;
            };
            let Some(sender) = blob.signer() else {
                tracing::debug!("Blob without a signer, happens if blob is version 0");
                continue;
            };
            // Peek the first share's payload (without consuming) to detect an
            // envelope, then eagerly authenticate the 24-byte fixed header so
            // `total_len()` is the authenticated `rollup_len` before any size gate
            // runs. Legacy blobs read nothing here, preserving their partial-read
            // savings.
            //
            // Load-bearing: `chunk()` must expose at least the 16-byte magic for any
            // real envelope (it returns the first share's whole payload, ~482 B). A
            // false negative would cache `Legacy` on first access while the guest
            // reclassifies the full accumulator as `Envelope` — a silent native/guest
            // divergence. Pinned by `first_share_chunk_exposes_envelope_magic`.
            let iter = blob.into_iter();
            let is_envelope = crate::envelope::has_magic_prefix(prost::bytes::Buf::chunk(&iter));
            let mut reader = CountedBufReader::new(iter);
            if is_envelope {
                reader.advance(crate::envelope::ENVELOPE_HEADER_LEN);
            }
            let blob_tx = BlobWithSender {
                blob: reader,
                range_in_namespace,
                sender,
                hash,
                envelope_state: OnceLock::new(),
            };
            output.push(blob_tx);
        }
        output
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct FilteredCelestiaBlock {
    pub(crate) header: CelestiaHeader,
    /// Batch related data.
    pub(crate) rollup_batch_data: NamespaceRelevantData,
    /// Proof related data.
    pub(crate) rollup_proof_data: NamespaceRelevantData,
}

#[cfg(feature = "native")]
impl sov_rollup_interface::node::da::SlotData for FilteredCelestiaBlock {
    type BlockHeader = CelestiaHeader;

    fn hash(&self) -> [u8; 32] {
        match self.header.header.hash() {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => {
                unreachable!("tendermint::Hash::None should not be possible")
            }
        }
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> sov_rollup_interface::da::Time {
        use sov_rollup_interface::da::BlockHeaderTrait;
        self.header.time()
    }
}

impl FilteredCelestiaBlock {
    #[cfg(feature = "native")]
    pub(crate) fn new(
        rollup_batch_data: NamespaceRelevantData,
        rollup_proof_data: NamespaceRelevantData,
        header: CelestiaHeader,
    ) -> anyhow::Result<Self> {
        Ok(FilteredCelestiaBlock {
            header,
            rollup_batch_data,
            rollup_proof_data,
        })
    }
}

/// Proof of namespace end boundary in the last relevant row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NamespaceBoundaryProof {
    /// Namespace proof for the boundary.
    /// For presence proofs this is narrowed to the last namespace share.
    /// For absence proofs this proves namespace absence in that row.
    pub last_share_proof: celestia_types::nmt::NamespaceProof,
    /// The last namespace share when `last_share_proof` is of presence; `None` for absence proofs.
    pub last_share: Option<celestia_types::Share>,
}

#[cfg(feature = "native")]
impl NamespaceBoundaryProof {
    pub(crate) fn from_namespace_data(namespace_data: &NamespaceRelevantData) -> Option<Self> {
        let last_row = namespace_data.data.rows().last()?;
        if last_row.shares.is_empty() && last_row.proof.is_of_presence() {
            panic!("Incorrect namespace data: last row proof is of presence, but no shares");
        } else if last_row.shares.is_empty() && last_row.proof.is_of_absence() {
            return Some(Self {
                last_share_proof: last_row.proof.clone(),
                last_share: None,
            });
        }
        let all_before_last = &last_row.shares[..last_row.shares.len().saturating_sub(1)];
        let last_share = last_row
            .shares
            .last()
            .expect("Incorrect namespace data: missing shares from last row")
            .clone();
        let last_share_proof = last_row
            .proof
            .narrow_range(all_before_last, &[], *namespace_data.namespace)
            .expect("Incorrect namespace data: cannot narrow range proof last share");
        Some(Self {
            last_share_proof: last_share_proof.into(),
            last_share: Some(last_share),
        })
    }
}

#[cfg(test)]
pub mod tests {
    use std::str::FromStr;

    use sov_rollup_interface::da::BlobReaderTrait;

    use super::BlobWithSender;
    use crate::envelope::{classify_and_decode, EnvelopeState};
    use crate::test_helper::files::*;
    use crate::test_helper::ROLLUP_BATCH_NAMESPACE;
    use crate::types::{NamespaceData, NamespaceRelevantData, TmHash};

    fn test_serialize_roundtrip(raw: [u8; 32]) {
        let tm_hash = TmHash::from(raw);
        let serde_serialized = serde_json::to_string(&tm_hash).unwrap();
        let serde_deserialized: TmHash = serde_json::from_str(&serde_serialized).unwrap();

        assert_eq!(tm_hash, serde_deserialized);

        let borsh_serialized = borsh::to_vec(&tm_hash).unwrap();
        let borsh_deserialized: TmHash = borsh::from_slice(&borsh_serialized).unwrap();

        assert_eq!(tm_hash, borsh_deserialized);
    }

    fn test_str_roundtrip(raw: [u8; 32]) {
        let tm_hash = TmHash::from(raw);
        let s = tm_hash.to_string();
        let restored = TmHash::from_str(&s).expect("TmHash::from_str failed");

        assert_eq!(tm_hash, restored);
    }

    #[test_strategy::proptest]
    fn proptest_str_roundtrip(raw: [u8; 32]) {
        test_str_roundtrip(raw);
    }

    #[test_strategy::proptest]
    fn proptest_serde_roundtrip(raw: [u8; 32]) {
        test_serialize_roundtrip(raw);
    }

    #[test]
    fn filtered_block_with_proof_data() {
        let block = with_rollup_proof_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_proof_data = block.rollup_proof_data;

        assert_eq!(rollup_proof_data.data.rows().len(), 1);
        assert_eq!(rollup_proof_data.data.rows()[0].shares.len(), 1);
        assert!(rollup_proof_data.data.rows()[0].proof.is_of_presence());
    }

    #[test]
    fn filtered_block_with_batch_data() {
        let block = with_rollup_batch_data::filtered_block();
        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_batch_data = block.rollup_batch_data;

        // single rollup share
        // assert_eq!(rollup_batch_data.group.shares().len(), 1);
        assert_eq!(rollup_batch_data.data.rows().len(), 1);
        assert_eq!(rollup_batch_data.data.rows()[0].shares.len(), 1);
        assert!(rollup_batch_data.data.rows()[0].proof.is_of_presence());
    }

    #[test]
    fn filtered_block_without_batch_data() {
        let block = without_rollup_batch_data::filtered_block();

        // valid dah
        block.header.validate_dah().unwrap();

        let rollup_batch_data = block.rollup_batch_data;

        // no rollup shares
        // we still get a single row, but with absence proof and no shares
        assert_eq!(rollup_batch_data.data.rows().len(), 1);
        assert_eq!(rollup_batch_data.data.rows()[0].shares.len(), 0);
        assert!(rollup_batch_data.data.rows()[0].proof.is_of_absence());
    }

    #[test]
    fn test_get_rollup_data() {
        let path = make_test_path(with_rollup_batch_data::DATA_PATH);
        let rows: NamespaceData = load_from_file(&path, ROLLUP_BATCH_ROWS_JSON).unwrap();

        let ns_data = NamespaceRelevantData::new(ROLLUP_BATCH_NAMESPACE, rows);

        let blobs = ns_data.get_blobs_with_sender();
        assert_eq!(1, blobs.len());
        let blob = &blobs[0];

        // this is a batch submitted by sequencer, consisting of a single
        // "CreateToken" transaction, but we verify only length there to
        // not make this test depend on deserialization logic
        assert_eq!(blob.total_len(), 277);
    }

    #[test]
    fn accessors_match_trait_view_for_raw_blobs() {
        let path = make_test_path(with_rollup_batch_data::DATA_PATH);
        let rows: NamespaceData = load_from_file(&path, ROLLUP_BATCH_ROWS_JSON).unwrap();

        let ns_data = NamespaceRelevantData::new(ROLLUP_BATCH_NAMESPACE, rows);

        let mut blob = ns_data.get_blobs_with_sender().remove(0);

        // For raw (uncompressed) blobs the DA and rollup views are identical,
        // both before and after a partial read.
        assert_eq!(blob.compressed_total_len(), blob.total_len());
        assert_eq!(blob.compressed_verified_data(), blob.verified_data());

        blob.advance(10);
        assert_eq!(blob.verified_data().len(), 10);
        assert_eq!(blob.compressed_verified_data(), blob.verified_data());
        assert_eq!(blob.compressed_total_len(), blob.total_len());
    }

    #[test]
    fn serde_roundtrip_drops_envelope_cache_and_recomputes() {
        let path = make_test_path(with_rollup_batch_data::DATA_PATH);
        let rows: NamespaceData = load_from_file(&path, ROLLUP_BATCH_ROWS_JSON).unwrap();
        let ns_data = NamespaceRelevantData::new(ROLLUP_BATCH_NAMESPACE, rows);

        let mut blob = ns_data.get_blobs_with_sender().remove(0);

        // Drive the live read path over the whole frame; this fills the
        // serde-skipped classification cache via the real accessors (no manual set).
        let total = blob.compressed_total_len();
        blob.advance(total);
        let expected_state = classify_and_decode(blob.compressed_verified_data());
        assert!(
            blob.envelope_state.get().is_some(),
            "the live read path fills the classification cache"
        );

        let json = serde_json::to_string(&blob).unwrap();
        assert!(
            !json.contains("envelope_state"),
            "skipped cache must not be serialized"
        );

        let restored: BlobWithSender = serde_json::from_str(&json).unwrap();

        // The cache is dropped on deserialize.
        assert_eq!(restored.envelope_state.get(), None);
        // Equality ignores the cache: populated original equals empty restored.
        assert_eq!(restored, blob);
        // Reconstruction from the authenticated bytes is deterministic.
        assert_eq!(
            classify_and_decode(restored.compressed_verified_data()),
            expected_state
        );
        // This fixture is a raw (non-envelope) blob.
        assert_eq!(expected_state, EnvelopeState::Legacy);
    }

    /// Build a [`BlobWithSender`] carrying `da_payload` as its on-DA payload, the same
    /// way `get_blobs_with_sender` does (real Celestia shares + eager header advance
    /// for envelopes). Lets us exercise the envelope read path without a devnet.
    fn blob_with_da_payload(da_payload: &[u8]) -> super::BlobWithSender {
        use prost::bytes::Buf;
        use sov_rollup_interface::common::HexHash;
        use sov_rollup_interface::da::CountedBufReader;

        use crate::verifier::address::CelestiaAddress;

        let signer = CelestiaAddress::from_str(crate::test_helper::ADDR_1).unwrap();
        let cblob = crate::test_helper::blob_from_data(
            ROLLUP_BATCH_NAMESPACE,
            da_payload.to_vec(),
            &signer,
        )
        .unwrap();
        let shares = cblob.to_shares().unwrap();
        let share_count = shares.len();
        let iter = crate::shares::Blob(shares).into_iter();
        let is_envelope = crate::envelope::has_magic_prefix(iter.chunk());
        let mut reader = CountedBufReader::new(iter);
        if is_envelope {
            reader.advance(crate::envelope::ENVELOPE_HEADER_LEN);
        }
        super::BlobWithSender {
            blob: reader,
            range_in_namespace: 0..share_count,
            sender: signer,
            hash: HexHash::new([0u8; 32]),
            envelope_state: std::sync::OnceLock::new(),
        }
    }

    /// A compressible, multi-chunk payload with an 8-byte period (LZ4 shrinks it) and
    /// distinct, asymmetric byte values.
    fn compressible_payload(len: usize) -> Vec<u8> {
        const PATTERN: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78];
        PATTERN.iter().copied().cycle().take(len).collect()
    }

    /// Pins the load-bearing assumption behind the eager-header read in
    /// `get_blobs_with_sender`: the first share's `Buf::chunk()` must expose at least the
    /// full envelope header. If it ever returned fewer than the 16-byte magic for a real
    /// envelope, native would cache `Legacy` on first access while the guest reclassifies
    /// the full accumulator as `Envelope` — a silent native/guest divergence.
    #[test]
    fn first_share_chunk_exposes_envelope_magic() {
        use prost::bytes::Buf;

        use crate::verifier::address::CelestiaAddress;

        // A real, multi-chunk compressed envelope spanning several Celestia shares.
        let rollup = compressible_payload(4000);
        let da_payload = crate::envelope::encode_for_submission(&rollup, true, 482);
        assert!(
            crate::envelope::has_magic_prefix(&da_payload),
            "fixture must be an envelope"
        );

        let signer = CelestiaAddress::from_str(crate::test_helper::ADDR_1).unwrap();
        let cblob = crate::test_helper::blob_from_data(ROLLUP_BATCH_NAMESPACE, da_payload, &signer)
            .unwrap();
        let shares = cblob.to_shares().unwrap();
        let iter = crate::shares::Blob(shares).into_iter();

        let first_chunk = Buf::chunk(&iter);
        assert!(
            first_chunk.len() >= crate::envelope::ENVELOPE_HEADER_LEN,
            "first share chunk must expose the whole fixed header, got {} bytes",
            first_chunk.len()
        );
        assert!(
            crate::envelope::has_magic_prefix(first_chunk),
            "eager-header detection must see the magic in the first chunk"
        );
    }

    #[test]
    fn compressed_envelope_round_trips_through_blob_with_sender() {
        let rollup = compressible_payload(4000);
        let da_payload = crate::envelope::encode_for_submission(&rollup, true, 482);
        assert!(
            crate::envelope::has_magic_prefix(&da_payload),
            "should be an envelope"
        );
        assert!(da_payload.len() < rollup.len(), "should compress");

        let mut blob = blob_with_da_payload(&da_payload);
        // `total_len` is the authenticated header `rollup_len`, known from the eager
        // header before the body is read.
        assert_eq!(blob.total_len(), rollup.len());
        // A full read decodes back to the original rollup payload.
        assert_eq!(blob.full_data(), rollup.as_slice());
        assert!(!blob.rollup_decode_failed());
    }

    #[test]
    fn corrupt_envelope_chunk_sets_rollup_decode_failed() {
        let rollup = compressible_payload(4000);
        let mut da_payload = crate::envelope::encode_for_submission(&rollup, true, 482);
        assert!(crate::envelope::has_magic_prefix(&da_payload));
        // Corrupt the first chunk's compressed body (past header + per-chunk framing).
        let body0 = crate::envelope::ENVELOPE_HEADER_LEN + crate::envelope::CHUNK_HEADER_LEN;
        da_payload[body0] ^= 0xFF;

        let mut blob = blob_with_da_payload(&da_payload);
        // Mode is header-only and immutable: `total_len` stays the header `rollup_len`.
        assert_eq!(blob.total_len(), rollup.len());
        let _ = blob.full_data();
        // Fully-present authenticated bytes that do not decode cleanly => slash signal.
        assert!(blob.rollup_decode_failed());
    }

    #[test]
    fn malformed_envelope_header_reads_as_raw() {
        // Magic prefix + invalid header (bad version), padded well past the 24-byte header.
        let mut da_payload = crate::envelope::ENVELOPE_MAGIC.to_vec();
        da_payload.push(2); // version = 2 (unsupported)
        da_payload.extend([0u8; 400]);

        let mut blob = blob_with_da_payload(&da_payload);
        // Malformed-as-raw: the rollup view is the authenticated DA bytes, so the
        // STF slashes it under legacy semantics rather than via `rollup_decode_failed`.
        assert_eq!(blob.total_len(), da_payload.len());
        assert_eq!(blob.full_data(), da_payload.as_slice());
        assert!(!blob.rollup_decode_failed());
    }

    #[test]
    fn partial_advance_reads_chunk_aligned_rollup_prefix() {
        let rollup = compressible_payload(4000);
        let da_payload = crate::envelope::encode_for_submission(&rollup, true, 200);
        assert!(
            crate::envelope::has_magic_prefix(&da_payload),
            "should be an envelope"
        );

        let mut blob = blob_with_da_payload(&da_payload);
        // Reading one rollup byte pulls whole chunks until >= 1 byte is covered.
        let prefix = blob.advance(1).to_vec();
        assert!(
            !prefix.is_empty() && prefix.len() < rollup.len(),
            "a chunk-aligned rollup prefix"
        );
        assert_eq!(
            prefix.as_slice(),
            &rollup[..prefix.len()],
            "prefix matches rollup"
        );
        // A genuine partial read (more DA bytes still available) is not a decode failure;
        // the verifier authenticates only the chunks consumed (the DoS bound).
        assert!(!blob.rollup_decode_failed());
        assert!(blob.compressed_verified_data().len() < blob.compressed_total_len());
    }

    /// Body-start offset of chunk index `k` in a DA frame, found by walking the
    /// per-chunk framing.
    fn nth_chunk_body_start(da_payload: &[u8], k: usize) -> usize {
        let mut off = crate::envelope::ENVELOPE_HEADER_LEN;
        for _ in 0..k {
            let enc = u16::from_le_bytes([da_payload[off + 2], da_payload[off + 3]]) as usize;
            off += crate::envelope::CHUNK_HEADER_LEN + enc;
        }
        off + crate::envelope::CHUNK_HEADER_LEN
    }

    #[test]
    fn incremental_advance_matches_wholesale() {
        // A compressible, multi-chunk envelope (small chunks => many chunks).
        let rollup = compressible_payload(4000);
        let da_payload = crate::envelope::encode_for_submission(&rollup, true, 200);
        assert!(
            crate::envelope::has_magic_prefix(&da_payload),
            "compressible payload should encode to an envelope"
        );

        // Read `a` incrementally in small steps; read `b` in one shot.
        let mut a = blob_with_da_payload(&da_payload);
        let mut b = blob_with_da_payload(&da_payload);

        // An intermediate small advance yields a chunk-aligned prefix of the rollup.
        let prefix = a.advance(700).to_vec();
        assert!(
            !prefix.is_empty() && prefix.len() < rollup.len(),
            "a chunk-aligned rollup prefix"
        );
        assert_eq!(prefix.as_slice(), &rollup[..prefix.len()]);

        // Finish `a` with more small advances; finish `b` in one `full_data`.
        for _ in 0..40 {
            if a.verified_data().len() >= a.total_len() {
                break;
            }
            a.advance(700);
        }
        let _ = b.full_data();

        // Incremental == wholesale, byte-for-byte.
        assert_eq!(a.verified_data(), rollup.as_slice());
        assert_eq!(a.verified_data(), b.verified_data());
        assert_eq!(a.total_len(), b.total_len());
        assert_eq!(a.rollup_decode_failed(), b.rollup_decode_failed());
        assert!(!a.rollup_decode_failed());
    }

    #[test]
    fn incremental_advance_matches_wholesale_across_dict_window() {
        // A payload several times the 64 KiB dict window: a 4 KiB pseudo-random block repeated so
        // each repeat is matchable only via the rolling dictionary. Reading it incrementally slides
        // the window (decoded prefix > DICT_WINDOW); a window-slide bug would corrupt the
        // incremental decode. Real batches are ~110-135 KiB, so this is the production case.
        let block: Vec<u8> = (0..4096u32)
            .map(|i| {
                let x = i.wrapping_mul(2_654_435_761);
                (x ^ (x >> 15)) as u8
            })
            .collect();
        let mut rollup = Vec::new();
        while rollup.len() < 3 * crate::envelope::DICT_WINDOW {
            rollup.extend_from_slice(&block);
        }
        let da_payload = crate::envelope::encode_for_submission(&rollup, true, 482);
        assert!(
            crate::envelope::has_magic_prefix(&da_payload),
            "repeated block should compress to an envelope"
        );

        let mut a = blob_with_da_payload(&da_payload);
        let mut b = blob_with_da_payload(&da_payload);

        // Drain `a` in many small steps that cross the window boundary; `b` in one shot.
        for _ in 0..2000 {
            let before = a.verified_data().len();
            a.advance(4096);
            if a.verified_data().len() >= a.total_len() || a.verified_data().len() == before {
                break;
            }
        }
        let _ = b.full_data();

        assert_eq!(
            a.verified_data().len(),
            rollup.len(),
            "incremental drain reads the whole rollup across the window"
        );
        assert_eq!(
            a.verified_data(),
            b.verified_data(),
            "incremental == wholesale across the sliding dict window"
        );
        assert!(!a.rollup_decode_failed());
    }

    #[test]
    fn incremental_advance_detects_corrupt_chunk_mid_stream() {
        let rollup = compressible_payload(4000);
        let mut da_payload = crate::envelope::encode_for_submission(&rollup, true, 200);
        assert!(crate::envelope::has_magic_prefix(&da_payload));
        // Corrupt the LZ4 token byte of chunk 5, so the decode fails only after several
        // clean chunks have already been decoded incrementally.
        let corrupt_at = nth_chunk_body_start(&da_payload, 5);
        da_payload[corrupt_at] ^= 0xFF;

        let mut a = blob_with_da_payload(&da_payload);
        let mut b = blob_with_da_payload(&da_payload);

        // Drive `a` incrementally (bounded loop); `b` in one shot.
        for _ in 0..40 {
            let before = a.verified_data().len();
            a.advance(300);
            if a.rollup_decode_failed() || a.verified_data().len() == before {
                break;
            }
        }
        let _ = b.full_data();

        assert!(
            a.rollup_decode_failed(),
            "incremental advance must surface a mid-stream corrupt chunk"
        );
        assert_eq!(a.rollup_decode_failed(), b.rollup_decode_failed());
        // Both expose the same clean prefix decoded before the corrupt chunk.
        assert_eq!(a.verified_data(), b.verified_data());
        assert_eq!(a.verified_data(), &rollup[..a.verified_data().len()]);
    }
}
