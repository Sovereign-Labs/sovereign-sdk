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

/// A streaming reader over one blob's authenticated DA-physical bytes that presents the
/// *logical* (post-decompression) view.
///
/// It owns the [`CountedBufReader`] whose accumulator is the unit of authentication — the
/// verifier, inclusion proofs, and all share-occupancy math operate over these
/// DA-physical/"compressed" bytes — and lazily derives the logical payload from that
/// authenticated prefix via [`classify_and_decode`], the single decoder shared by native
/// reads and the guest verifier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EnvelopeReader {
    /// The authenticated DA-physical reader. Its accumulator is the only serialized state.
    compressed: CountedBufReader<BlobIterator>,

    /// Interior, serde-skipped cache of the blob's envelope classification and decode.
    ///
    /// Derived only from the authenticated DA-physical accumulator
    /// ([`Self::compressed_verified_data`]); never a serialized witness claim. Empty after
    /// construction and after every (de)serialization, then recomputed deterministically by
    /// [`Self::logical_state`] (holding only `&self`). Ignored by `PartialEq`.
    #[serde(skip)]
    state: OnceLock<EnvelopeState>,
}

impl PartialEq for EnvelopeReader {
    fn eq(&self, other: &Self) -> bool {
        // The envelope cache is derived state, not identity: two readers over the same
        // authenticated bytes are equal regardless of whether either has lazily computed its
        // classification yet.
        self.compressed == other.compressed
    }
}

/// `EnvelopeReader` distinguishes the two byte streams a blob represents:
///
/// * **DA-physical ("compressed")**: the payload bytes as actually posted to Celestia.
///   Share-occupancy math (inclusion proofs, namespace continuity) is defined over these
///   bytes and only these bytes.
/// * **Logical**: the payload bytes exposed to the rollup via [`BlobReaderTrait`].
///
/// For legacy (non-envelope) blobs the two streams are identical; once a blob is posted in a
/// compressed envelope they diverge, and proof generation/verification keep using the
/// `compressed_*` accessors so the share math stays tied to what is actually on DA.
impl EnvelopeReader {
    /// Wrap a blob's share iterator. For magic-prefixed blobs the fixed envelope header is
    /// eagerly authenticated so the logical length and classification are pinned before any
    /// size gate reads `total_len()`; legacy blobs read nothing here, preserving the
    /// "a deferred blob proves no shares" DoS property.
    #[cfg(feature = "native")]
    pub(crate) fn new(iter: BlobIterator) -> Self {
        // Peek the leading bytes (the first share's payload) to detect the envelope magic
        // without consuming anything for legacy blobs.
        let has_magic = crate::envelope::has_magic_prefix(prost::bytes::Buf::chunk(&iter));
        let mut compressed = CountedBufReader::new(iter);
        if has_magic {
            // Eagerly authenticate the fixed envelope header so the logical length and
            // classification are pinned before any size gate reads `total_len()`.
            compressed.advance(crate::envelope::ENVELOPE_HEADER_LEN);
        }
        Self {
            compressed,
            state: OnceLock::new(),
        }
    }

    /// DA-physical payload bytes consumed so far. These are the bytes that inclusion proofs
    /// must cover.
    pub(crate) fn compressed_verified_data(&self) -> &[u8] {
        self.compressed.accumulator()
    }

    /// Total DA-physical payload length. Must always equal the `sequence_length` recorded in
    /// the blob's first share; the verifier enforces this.
    pub(crate) fn compressed_total_len(&self) -> usize {
        self.compressed.total_len()
    }

    /// The blob's envelope classification + decode over the authenticated accumulator,
    /// computed once and cached. Recomputed after (de)serialization (the cache is
    /// `#[serde(skip)]`) and after a native read extends the accumulator.
    fn logical_state(&self) -> &EnvelopeState {
        self.state
            .get_or_init(|| classify_and_decode(self.compressed_verified_data()))
    }

    /// Logical payload bytes observed by the rollup so far. For a compressed envelope these
    /// are the bytes decoded from the complete chunks in the authenticated prefix; for
    /// legacy/malformed blobs the logical and DA-physical bytes coincide.
    pub(crate) fn logical_verified_data(&self) -> &[u8] {
        match self.logical_state() {
            EnvelopeState::Envelope(decoded) => &decoded.logical,
            EnvelopeState::Legacy | EnvelopeState::Malformed => self.compressed_verified_data(),
        }
    }

    /// Total length of the logical payload exposed to the rollup. For an envelope this is the
    /// authenticated header `logical_len` (never the currently-decodable length, so it stays
    /// stable as the blob is read); otherwise the DA-physical length.
    pub(crate) fn logical_total_len(&self) -> usize {
        match self.logical_state() {
            EnvelopeState::Envelope(decoded) => decoded.header.logical_len as usize,
            EnvelopeState::Legacy | EnvelopeState::Malformed => self.compressed_total_len(),
        }
    }

    /// Whether the fully-provided authenticated bytes fail to decode into a clean, exact,
    /// complete logical payload (see [`BlobReaderTrait::logical_decode_failed`]).
    pub(crate) fn logical_decode_failed(&self) -> bool {
        let EnvelopeState::Envelope(decoded) = self.logical_state() else {
            // Legacy and malformed-as-raw blobs expose their authenticated bytes
            // verbatim; there is no decode step that can fail.
            return false;
        };
        // A structurally invalid chunk (cap violation, bad LZ4, raw length mismatch, or
        // logical-sum overrun) is the sender's fault regardless of how much was read.
        if !decoded.clean {
            return true;
        }
        let total = self.compressed_total_len();
        if decoded.logical.len() == decoded.header.logical_len as usize {
            // All declared logical bytes decoded. The encoding is canonical only if the
            // chunks consumed the entire authenticated posted payload — a trailing physical
            // tail is non-canonical. `total` is the authenticated sequence length, so we
            // flag (and slash) the tail WITHOUT reading it: a tiny-logical/huge-tail blob
            // cannot force the verifier to authenticate the tail just to reject it.
            decoded.consumed != total
        } else {
            // Fewer logical bytes than declared. This is a sender fault (too few or
            // truncated chunks) only once the whole posted payload is present; a genuine
            // partial read (more physical still available) is the prover-withholding case,
            // which sov-blob-storage's unconditional completeness assert fails closed.
            self.compressed_verified_data().len() == total
        }
    }

    /// Native incremental read: authenticate whole chunks until at least `num_bytes` more
    /// logical bytes are covered, then re-derive the cached logical view with the single
    /// decoder ([`classify_and_decode`]).
    ///
    /// The pull loop only decides *how many DA-physical bytes to authenticate*: it walks the
    /// chunk framing, validating each chunk's framing BEFORE authenticating its
    /// attacker-controlled-length payload and never reading past logical completion. So a
    /// partial read authenticates only a prefix of the physical blob (the DoS property) and a
    /// tiny-logical/huge-tail blob never forces the tail to be read. Decoding itself is left
    /// entirely to `classify_and_decode`, so native and guest share one decoder.
    ///
    /// Re-decoding the authenticated prefix is O(prefix) per call; consumers read a blob once
    /// via `full_data()`, so this is O(n) overall. Only a pathological stream of tiny
    /// `advance` calls would be quadratic, which no consumer does.
    #[cfg(feature = "native")]
    pub(crate) fn advance(&mut self, num_bytes: usize) -> &[u8] {
        let (codec, logical_len, already) = match self.logical_state() {
            EnvelopeState::Envelope(decoded) => (
                decoded.header.codec,
                decoded.header.logical_len,
                decoded.logical.len(),
            ),
            EnvelopeState::Legacy | EnvelopeState::Malformed => {
                // Legacy/malformed: logical and DA-physical bytes coincide.
                self.compressed.advance(num_bytes);
                return self.logical_verified_data();
            }
        };

        // Pull whole chunks (framing only — no decode) until at least `num_bytes` more logical
        // bytes are covered. The accumulator always ends on a chunk boundary, so the next
        // chunk's 4-byte framing starts at its current length.
        let mut covered = already;
        let target = covered.saturating_add(num_bytes).min(logical_len as usize);
        while covered < target {
            let pos = self.compressed.accumulator().len();
            if pos >= self.compressed.total_len() {
                break; // physical EOF
            }
            self.compressed.advance(crate::envelope::CHUNK_HEADER_LEN);
            let acc = self.compressed.accumulator();
            if acc.len() < pos + crate::envelope::CHUNK_HEADER_LEN {
                break; // truncated framing at EOF
            }
            let chunk_logical = u16::from_le_bytes([acc[pos], acc[pos + 1]]);
            let chunk_encoded = u16::from_le_bytes([acc[pos + 2], acc[pos + 3]]);
            // Validate framing BEFORE authenticating the (attacker-controlled-length) payload,
            // so a malicious framing can't make a 1-byte logical read authenticate a u16-sized
            // payload. A structurally bad chunk stops the pull; `classify_and_decode` then
            // marks the decode unclean and `logical_decode_failed()` slashes.
            if !crate::envelope::chunk_framing_valid(
                codec,
                chunk_logical,
                chunk_encoded,
                covered,
                logical_len,
            ) {
                break;
            }
            self.compressed.advance(chunk_encoded as usize);
            if self.compressed.accumulator().len()
                < pos + crate::envelope::CHUNK_HEADER_LEN + chunk_encoded as usize
            {
                break; // truncated payload at EOF
            }
            covered = covered.saturating_add(chunk_logical as usize);
        }

        // Single source of truth for the logical view: re-derive from the authenticated
        // prefix with the one decoder the guest also uses.
        self.state = OnceLock::new();
        let _ = self
            .state
            .set(classify_and_decode(self.compressed_verified_data()));
        self.logical_verified_data()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobWithSender {
    pub(crate) blob: EnvelopeReader,
    // Range in the entire namespace
    pub(crate) range_in_namespace: Range<usize>,
    pub(crate) sender: CelestiaAddress,
    pub hash: HexHash,
}

impl PartialEq for BlobWithSender {
    fn eq(&self, other: &Self) -> bool {
        self.blob == other.blob
            && self.range_in_namespace == other.range_in_namespace
            && self.sender == other.sender
            && self.hash == other.hash
    }
}

impl BlobWithSender {
    /// DA-physical payload bytes consumed so far; see
    /// [`EnvelopeReader::compressed_verified_data`]. Used by the verifier and proof
    /// generation, which operate over the posted bytes.
    pub(crate) fn compressed_verified_data(&self) -> &[u8] {
        self.blob.compressed_verified_data()
    }

    /// Total DA-physical payload length; see [`EnvelopeReader::compressed_total_len`].
    pub(crate) fn compressed_total_len(&self) -> usize {
        self.blob.compressed_total_len()
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
        self.blob.logical_verified_data()
    }

    fn total_len(&self) -> usize {
        self.blob.logical_total_len()
    }

    fn logical_decode_failed(&self) -> bool {
        self.blob.logical_decode_failed()
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.blob.advance(num_bytes)
    }

    // `full_data()` uses the trait default: `advance(total_len() - verified_data().len())`.
    // For an envelope that is `advance(logical_len - covered)`, which authenticates whole
    // chunks up to logical completion and STOPS — it must not authenticate a trailing physical
    // tail (a tiny-logical/huge-tail blob would otherwise force full-blob verification just to
    // slash it). For legacy/malformed blobs it advances over all remaining physical bytes.
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
            let blob_tx = BlobWithSender {
                blob: EnvelopeReader::new(blob.into_iter()),
                range_in_namespace,
                sender,
                hash,
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

        // For raw (uncompressed) blobs the DA-physical and logical views are identical,
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
        use super::BlobWithSender;
        use crate::envelope::{classify_and_decode, EnvelopeState};

        let path = make_test_path(with_rollup_batch_data::DATA_PATH);
        let rows: NamespaceData = load_from_file(&path, ROLLUP_BATCH_ROWS_JSON).unwrap();
        let ns_data = NamespaceRelevantData::new(ROLLUP_BATCH_NAMESPACE, rows);

        let mut blob = ns_data.get_blobs_with_sender().remove(0);

        // Reading the whole blob lazily populates the envelope cache.
        blob.full_data();
        assert!(
            blob.blob.state.get().is_some(),
            "reading the blob should populate the cache"
        );
        let expected_state = classify_and_decode(blob.compressed_verified_data());

        let json = serde_json::to_string(&blob).unwrap();
        assert!(
            !json.contains("\"state\""),
            "skipped cache must not be serialized"
        );

        let restored: BlobWithSender = serde_json::from_str(&json).unwrap();

        // The cache is dropped on deserialize.
        assert_eq!(restored.blob.state.get(), None);
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
}

/// Integration tests for the envelope read path, exercised over *real* Celestia v1 shares
/// built from arbitrary physical bytes (`Blob::new(..).to_shares()`), so the accessors run
/// against genuine multi-share blob iteration exactly as in production.
#[cfg(test)]
mod envelope_blob_tests {
    use std::str::FromStr;

    use sov_rollup_interface::common::HexHash;
    use sov_rollup_interface::da::BlobReaderTrait;

    use super::{BlobWithSender, EnvelopeReader};
    use crate::envelope::{
        classify_and_decode, encode_for_submission, EnvelopeState, CODEC_LZ4, CODEC_RAW_CHUNK,
        ENVELOPE_HEADER_LEN, ENVELOPE_MAGIC, ENVELOPE_VERSION, MAX_LOGICAL_CHUNK_LEN,
    };
    use crate::test_helper::{ADDR_1, ROLLUP_BATCH_NAMESPACE};
    use crate::verifier::address::CelestiaAddress;

    /// Build a [`BlobWithSender`] over `physical` exactly as `get_blobs_with_sender` would:
    /// genuine Celestia v1 shares wrapped by [`EnvelopeReader::new`] (which eagerly
    /// authenticates the fixed header for magic-prefixed blobs).
    fn blob_over_physical(physical: &[u8]) -> BlobWithSender {
        let signer = CelestiaAddress::from_str(ADDR_1).unwrap();
        let json =
            celestia_types::Blob::new(ROLLUP_BATCH_NAMESPACE, physical.to_vec(), Some(signer.0))
                .expect("valid v1 blob");
        let shares = json.to_shares().expect("blob splits into shares");
        let range_in_namespace = 0..shares.len();
        BlobWithSender {
            blob: EnvelopeReader::new(crate::shares::Blob(shares).into_iter()),
            range_in_namespace,
            sender: signer,
            hash: HexHash::new([0u8; 32]),
        }
    }

    /// A raw-codec frame with an explicit (possibly inconsistent) header `logical_len` and
    /// hand-built chunks — for crafting non-canonical / structurally invalid payloads.
    fn raw_frame(logical_len: u32, chunks: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut v = ENVELOPE_MAGIC.to_vec();
        v.push(ENVELOPE_VERSION);
        v.push(CODEC_RAW_CHUNK);
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&logical_len.to_le_bytes());
        for (clen, payload) in chunks {
            v.extend_from_slice(&clen.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
        }
        v
    }

    #[test]
    fn canonical_envelope_full_read_exposes_logical() {
        // Compressible and large enough to span many chunks across several shares.
        let logical = vec![0x5Au8; 2000];
        let physical = encode_for_submission(&logical, true, 512).unwrap();
        assert!(physical.len() < logical.len(), "fixture should compress");

        let mut blob = blob_over_physical(&physical);
        // `total_len` is the logical length read from the authenticated header.
        assert_eq!(blob.total_len(), logical.len());
        // A full read decodes every chunk to the complete logical payload.
        assert_eq!(blob.full_data(), &logical[..]);
        assert!(!blob.logical_decode_failed());
    }

    #[test]
    fn partial_advance_reads_only_covering_chunks() {
        // Several chunks; highly compressible so each chunk is tiny on the wire.
        let logical = vec![0x5Au8; 2000];
        let physical = encode_for_submission(&logical, true, 512).unwrap();
        let mut blob = blob_over_physical(&physical);
        let full_physical = blob.compressed_total_len();

        // Ask for 1 logical byte -> only the first chunk is pulled in, not the whole blob.
        blob.advance(1);
        assert!(!blob.verified_data().is_empty());
        assert!(
            blob.verified_data().len() <= MAX_LOGICAL_CHUNK_LEN as usize,
            "rounds up to one chunk, not the whole payload"
        );
        assert!(
            blob.compressed_verified_data().len() < full_physical,
            "a partial read authenticates only a prefix of the physical blob (DoS property)"
        );
        assert!(
            !blob.logical_decode_failed(),
            "a clean partial prefix is not a decode failure"
        );

        // Reading on yields the complete logical payload.
        assert_eq!(blob.full_data(), &logical[..]);
    }

    #[test]
    fn repeated_partial_advances_extend_envelope_state() {
        let logical = vec![0x5Au8; 2000];
        let physical = encode_for_submission(&logical, true, 512).unwrap();
        let mut blob = blob_over_physical(&physical);

        blob.advance(1);
        let EnvelopeState::Envelope(first) = blob
            .blob
            .state
            .get()
            .expect("first partial read initializes envelope state")
        else {
            panic!("expected envelope state");
        };
        let first_consumed = first.consumed;
        let first_logical_len = first.logical.len();
        assert_eq!(blob.verified_data().len(), first_logical_len);

        blob.advance(1);
        let EnvelopeState::Envelope(second) = blob
            .blob
            .state
            .get()
            .expect("second partial read keeps envelope state")
        else {
            panic!("expected envelope state");
        };
        assert!(second.clean);
        assert!(second.consumed > first_consumed);
        assert!(second.logical.len() > first_logical_len);
        assert_eq!(blob.verified_data().len(), second.logical.len());
    }

    #[test]
    fn tiny_logical_with_large_tail_is_slashed_without_reading_tail() {
        // A valid small-logical envelope followed by a large trailing physical tail. The
        // logical size/gas gate sees only the small declared length; a full read must
        // decode to logical completion and STOP, so the tail is never authenticated — yet
        // the blob is still flagged non-canonical and slashed.
        let logical = vec![0x5Au8; 100];
        let mut physical = encode_for_submission(&logical, true, 512).unwrap();
        let canonical_len = physical.len();
        physical.extend(std::iter::repeat_n(0xEEu8, 50_000));

        let mut blob = blob_over_physical(&physical);
        assert_eq!(
            blob.total_len(),
            logical.len(),
            "gate sees only logical_len"
        );

        blob.full_data();
        assert!(
            blob.compressed_verified_data().len() <= canonical_len,
            "a full read must not pull in the trailing tail (DoS guard)"
        );
        assert!(
            blob.logical_decode_failed(),
            "chunks don't consume the posted tail -> non-canonical -> slash"
        );
    }

    #[test]
    fn advance_does_not_authenticate_oversized_chunk_payload() {
        // Valid header, but the first chunk's framing claims an encoded length far above the
        // cap. A partial read must reject on the framing, before authenticating the payload.
        let mut physical = ENVELOPE_MAGIC.to_vec();
        physical.push(ENVELOPE_VERSION);
        physical.push(CODEC_LZ4);
        physical.extend_from_slice(&0u16.to_le_bytes()); // flags
        physical.extend_from_slice(&100u32.to_le_bytes()); // logical_len
        physical.extend_from_slice(&1u16.to_le_bytes()); // chunk_logical_len = 1
        physical.extend_from_slice(&50_000u16.to_le_bytes()); // chunk_encoded_len >> cap
        physical.extend(std::iter::repeat_n(0u8, 50_000)); // the oversized payload

        let mut blob = blob_over_physical(&physical);
        blob.advance(1);
        assert!(
            blob.compressed_verified_data().len() <= ENVELOPE_HEADER_LEN + 4,
            "only header + framing read; the oversized payload is never authenticated"
        );
        assert!(
            blob.logical_decode_failed(),
            "an over-cap chunk framing is structurally invalid -> slash"
        );
    }

    #[test]
    fn deferred_envelope_reads_only_header_but_knows_logical_len() {
        let logical = vec![0x5Au8; 2000];
        let physical = encode_for_submission(&logical, true, 512).unwrap();

        let blob = blob_over_physical(&physical);
        // Only the 24-byte header has been read (DoS: a size-checked blob proves ~1 share).
        assert_eq!(blob.compressed_verified_data().len(), ENVELOPE_HEADER_LEN);
        // Yet the logical length is already known from the authenticated header.
        assert_eq!(blob.total_len(), logical.len());
        assert!(blob.verified_data().is_empty(), "no logical bytes read yet");
        // A header-only prefix is a clean partial read, not a decode failure.
        assert!(!blob.logical_decode_failed());
    }

    #[test]
    fn trailing_bytes_after_canonical_envelope_fail_decode() {
        let logical = vec![0x5Au8; 1500];
        let mut physical = encode_for_submission(&logical, true, 512).unwrap();
        physical.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let mut blob = blob_over_physical(&physical);
        blob.full_data();
        // The chunks decode cleanly to `logical`, but they do not consume the trailing
        // bytes, so the full physical payload is not a canonical encoding -> slash.
        assert_eq!(blob.verified_data(), &logical[..]);
        assert!(blob.logical_decode_failed());
    }

    #[test]
    fn too_few_chunks_fail_decode() {
        // Header claims 1000 logical bytes but the chunks supply only 300.
        let physical = raw_frame(1000, &[(300u16, vec![7u8; 300])]);
        let mut blob = blob_over_physical(&physical);
        blob.full_data();
        assert_eq!(blob.total_len(), 1000);
        assert_eq!(blob.verified_data().len(), 300);
        assert!(
            blob.logical_decode_failed(),
            "sum(chunk_logical_len) < logical_len is non-canonical"
        );
    }

    #[test]
    fn over_cap_chunk_fails_decode() {
        let physical = raw_frame(
            MAX_LOGICAL_CHUNK_LEN as u32 + 1,
            &[(
                MAX_LOGICAL_CHUNK_LEN + 1,
                vec![1u8; MAX_LOGICAL_CHUNK_LEN as usize + 1],
            )],
        );
        let mut blob = blob_over_physical(&physical);
        blob.full_data();
        assert!(
            blob.logical_decode_failed(),
            "a structurally invalid (over-cap) chunk must slash"
        );
    }

    #[test]
    fn corrupt_lz4_chunk_body_is_slashed() {
        // Valid header and valid chunk framing, but the LZ4 body cannot decode to the
        // declared `chunk_logical_len`. The framing-only pull still authenticates the chunk
        // (it can't see LZ4 corruption); the single decoder then marks the envelope unclean,
        // so `logical_decode_failed()` slashes — identically in native and guest, since both
        // decode via `classify_and_decode`.
        let mut physical = ENVELOPE_MAGIC.to_vec();
        physical.push(ENVELOPE_VERSION);
        physical.push(CODEC_LZ4);
        physical.extend_from_slice(&0u16.to_le_bytes()); // flags
        physical.extend_from_slice(&100u32.to_le_bytes()); // logical_len = 100
        physical.extend_from_slice(&100u16.to_le_bytes()); // chunk_logical_len = 100 (<= cap)
        physical.extend_from_slice(&1u16.to_le_bytes()); // chunk_encoded_len = 1 (valid framing)
        physical.push(0x00); // a 1-byte "LZ4 block" cannot expand to 100 bytes -> decode error

        let mut blob = blob_over_physical(&physical);
        blob.full_data();
        assert!(
            blob.logical_decode_failed(),
            "valid framing but an undecodable LZ4 body must slash"
        );
    }

    #[test]
    fn serde_roundtrip_recomputes_envelope_decode() {
        let logical = vec![0x5Au8; 2000];
        let physical = encode_for_submission(&logical, true, 512).unwrap();

        let mut blob = blob_over_physical(&physical);
        blob.full_data();

        let json = serde_json::to_string(&blob).unwrap();
        let restored: BlobWithSender = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.blob.state.get(),
            None,
            "cache dropped on deserialize"
        );
        assert_eq!(restored, blob);
        // The decode recomputed from the authenticated bytes is the full logical payload.
        match classify_and_decode(restored.compressed_verified_data()) {
            EnvelopeState::Envelope(decoded) => assert_eq!(decoded.logical, logical),
            other => panic!("expected Envelope, got {other:?}"),
        }
    }
}
