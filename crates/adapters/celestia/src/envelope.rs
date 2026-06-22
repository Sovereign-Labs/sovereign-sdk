//! Celestia compressed-blob envelope format (v1) parsing, decoding and encoding.
//!
//! A rollup blob may be posted to Celestia either as today's raw payload or,
//! once compression lands, wrapped in a fixed-header *envelope*. This module
//! holds the pure, deterministic logic that recognizes, validates and decodes
//! that envelope from a blob's authenticated DA bytes.
//!
//! Everything on the read/decode path is intentionally side-effect free and
//! ungated: the guest verifier and native extraction must decode identical bytes
//! identically, so the same compiled functions run in both. Only the *encoder*
//! (used when this node submits a blob) is `native`-gated.
//!
//! ## Envelope format v1 (chunked)
//!
//! ```text
//! fixed header (24 bytes):
//!   magic        [16] b"SOV_CELESTIA_CMP"
//!   version      [1]  = 1
//!   codec        [1]  0 = raw chunk (escape); 1 = LZ4 block
//!   flags        [2]  must be 0
//!   rollup_len   [4]  u32 LE, total decoded length, <= MAX_ROLLUP_BLOB_LEN
//! then a sequence of independently-decodable chunks (the payload):
//!   chunk_rollup_len [2]  u16 LE, 1..=MAX_ROLLUP_CHUNK_LEN
//!   chunk_encoded_len [2]  u16 LE, 1..=MAX_COMPRESSED_CHUNK_LEN
//!   chunk_payload     [chunk_encoded_len]
//! ```
//!
//! Chunks are decodable on their own, so reading the first N rollup bytes only
//! authenticates and decodes the chunks covering them: verifier/guest work scales
//! with bytes consumed, not blob size.

/// Envelope magic prefix. 128 bits, chosen so a standard Borsh batch/proof
/// payload beginning with these exact bytes would already be malformed in
/// practice.
pub(crate) const ENVELOPE_MAGIC: [u8; 16] = *b"SOV_CELESTIA_CMP";

/// The only recognized envelope format version.
pub(crate) const ENVELOPE_VERSION: u8 = 1;

/// Raw-chunk codec: each chunk is the rollup bytes verbatim
/// (`chunk_encoded_len == chunk_rollup_len`). Exists mainly to escape payloads
/// that naturally begin with [`ENVELOPE_MAGIC`].
pub(crate) const CODEC_RAW_ESCAPE: u8 = 0;

/// LZ4-block codec. Each chunk is an independent LZ4 block decodable without any
/// later chunk, using the framing-supplied output size.
pub(crate) const CODEC_LZ4: u8 = 1;

/// Fixed header size: magic[16] + version[1] + codec[1] + flags[2] + rollup_len[4].
pub(crate) const ENVELOPE_HEADER_LEN: usize = 24;

/// Per-chunk framing size: chunk_rollup_len[2] + chunk_encoded_len[2].
pub(crate) const CHUNK_HEADER_LEN: usize = 4;

/// Hard cap on the decoded rollup payload length.
///
/// 64 MiB sits comfortably above any realistic rollup batch (the per-tx
/// `MAX_TX_SIZE` ceiling is 8 MiB) yet well below the ~126 MiB a Celestia data
/// square can physically hold, so a malicious `rollup_len` cannot request an
/// unbounded allocation.
pub(crate) const MAX_ROLLUP_BLOB_LEN: u32 = 64 * 1024 * 1024;

/// Maximum UNCOMPRESSED (decoded) length of a single chunk — the decompression-bomb bound.
/// Sourced from `CELESTIA_MAX_ROLLUP_CHUNK_LEN` in `constants.toml` (16 KiB) so an operator can
/// tune it without an SDK release. The verifier allocates at most this many bytes per chunk and
/// rejects any chunk claiming to decode larger; it also sets the partial-read granularity. The
/// default chunk size (`config::default_compression_chunk_size`, one share = 482) stays
/// conservative; advanced operators may opt up to this cap.
///
/// Consensus-relevant: every reader must agree on it, but it is safe to set before compression is
/// ever emitted on a live network. Typed `u16`, so a `constants.toml` value above 65535 is a
/// compile error — and it must stay well below that for the LZ4 worst-case framing to fit `u16`.
pub(crate) const MAX_ROLLUP_CHUNK_LEN: u16 =
    sov_modules_macros::config_value!("CELESTIA_MAX_ROLLUP_CHUNK_LEN");

/// Maximum COMPRESSED (on-DA) length of a single chunk — must fit three signed Celestia shares.
/// Sourced from `CELESTIA_MAX_COMPRESSED_CHUNK_SIZE` in `constants.toml` (1394). Bounds the shares
/// a single-chunk read forces the prover to authenticate in ZK, i.e. the DoS cost of spam.
/// Consensus-relevant — same rules as [`MAX_ROLLUP_CHUNK_LEN`].
pub(crate) const MAX_COMPRESSED_CHUNK_LEN: u16 =
    sov_modules_macros::config_value!("CELESTIA_MAX_COMPRESSED_CHUNK_SIZE");

// Header field offsets within the fixed header.
const OFFSET_VERSION: usize = 16;
const OFFSET_CODEC: usize = 17;
const OFFSET_FLAGS: usize = 18; // 2 bytes, u16 LE
const OFFSET_ROLLUP_LEN: usize = 20; // 4 bytes, u32 LE

/// The parsed and validated fixed envelope header. `version` and `flags` are
/// validated against their only legal values during parsing, so only the fields
/// that drive decoding are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnvelopeHeader {
    /// Codec id; one of [`CODEC_RAW_ESCAPE`] or [`CODEC_LZ4`].
    pub codec: u8,
    /// Declared total decoded payload length.
    pub rollup_len: u32,
}

/// The result of decoding the chunk stream of a valid-header envelope from a
/// (possibly partial) authenticated prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecodedEnvelope {
    /// The authenticated fixed header.
    pub header: EnvelopeHeader,
    /// Rollup bytes produced by the complete chunks decoded so far.
    pub rollup: Vec<u8>,
    /// DA bytes consumed by the header plus the complete chunks decoded
    /// (a partial trailing chunk, or bytes past rollup completion, are left
    /// unconsumed).
    pub consumed: usize,
    /// `false` iff the decoder hit a structural error (cap violation, running-sum
    /// overrun, raw-codec length mismatch, or LZ4 failure). A clean decode that is
    /// merely incomplete (a truncated trailing chunk on a partial read) stays
    /// `true`.
    pub clean: bool,
}

#[cfg(feature = "native")]
impl DecodedEnvelope {
    /// True iff this is a canonical decode of a `frame_len`-byte frame: clean, the
    /// chunks consumed the whole frame, and the decoded length equals the declared
    /// `rollup_len`. It does not check *which* bytes decoded — callers that must pin
    /// the exact payload add `&& self.rollup == expected`.
    fn is_canonical_for_frame_len(&self, frame_len: usize) -> bool {
        self.clean
            && self.consumed == frame_len
            && self.rollup.len() == self.header.rollup_len as usize
    }
}

/// Classification of a blob's authenticated DA bytes.
///
/// Derived purely from the bytes, so it is identical on native and guest builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvelopeState {
    /// No magic prefix: a legacy raw blob, today's behavior.
    Legacy,
    /// Magic prefix present but the fixed header is invalid. Treated as
    /// authenticated raw bytes (and therefore slashable) on the read path.
    Malformed,
    /// A valid v1 envelope header, with the chunks decoded from the available
    /// authenticated prefix.
    Envelope(DecodedEnvelope),
}

/// Why a fixed header failed validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EnvelopeError {
    /// The frame is shorter than the fixed 24-byte header.
    #[error("envelope frame too short for header: {len} bytes")]
    TooShort {
        /// Actual frame length.
        len: usize,
    },
    /// The frame does not begin with [`ENVELOPE_MAGIC`].
    #[error("envelope frame missing magic prefix")]
    MissingMagic,
    /// The version byte is not [`ENVELOPE_VERSION`].
    #[error("unsupported envelope version: {0}")]
    BadVersion(u8),
    /// The reserved flags are nonzero.
    #[error("nonzero envelope flags: {0:#06x}")]
    BadFlags(u16),
    /// The codec id is not a known v1 codec.
    #[error("unsupported codec id: {0}")]
    UnsupportedCodec(u8),
    /// The declared `rollup_len` exceeds [`MAX_ROLLUP_BLOB_LEN`].
    #[error("rollup_len {rollup_len} exceeds cap")]
    RollupLenTooLarge {
        /// The offending declared length.
        rollup_len: u32,
    },
}

/// Returns true iff `buf` begins with the 16-byte envelope magic.
pub(crate) fn has_magic_prefix(buf: &[u8]) -> bool {
    buf.starts_with(&ENVELOPE_MAGIC)
}

/// Validate and parse the fixed 24-byte header. This does *not* look at the chunk
/// stream — chunk framing is validated lazily, per chunk, during decode.
pub(crate) fn parse_header(buf: &[u8]) -> Result<EnvelopeHeader, EnvelopeError> {
    if !has_magic_prefix(buf) {
        return Err(EnvelopeError::MissingMagic);
    }
    if buf.len() < ENVELOPE_HEADER_LEN {
        return Err(EnvelopeError::TooShort { len: buf.len() });
    }

    let version = buf[OFFSET_VERSION];
    if version != ENVELOPE_VERSION {
        return Err(EnvelopeError::BadVersion(version));
    }

    let codec = buf[OFFSET_CODEC];
    if codec != CODEC_RAW_ESCAPE && codec != CODEC_LZ4 {
        return Err(EnvelopeError::UnsupportedCodec(codec));
    }

    let flags = u16::from_le_bytes([buf[OFFSET_FLAGS], buf[OFFSET_FLAGS + 1]]);
    if flags != 0 {
        return Err(EnvelopeError::BadFlags(flags));
    }

    let rollup_len = u32::from_le_bytes([
        buf[OFFSET_ROLLUP_LEN],
        buf[OFFSET_ROLLUP_LEN + 1],
        buf[OFFSET_ROLLUP_LEN + 2],
        buf[OFFSET_ROLLUP_LEN + 3],
    ]);
    if rollup_len > MAX_ROLLUP_BLOB_LEN {
        return Err(EnvelopeError::RollupLenTooLarge { rollup_len });
    }

    Ok(EnvelopeHeader { codec, rollup_len })
}

/// Validate a single chunk's framing *before* its (attacker-controlled-length)
/// payload is read or allocated. Shared by the decoder and the native
/// whole-chunk `advance`, so both bound the bytes a chunk read authenticates
/// identically.
///
/// `covered` is the running sum of `chunk_rollup_len` decoded so far;
/// `rollup_len` is the header's declared total.
pub(crate) fn chunk_framing_valid(
    codec: u8,
    chunk_rollup: u16,
    chunk_encoded: u16,
    covered: usize,
    rollup_len: usize,
) -> bool {
    // Zero-length chunks are rejected: otherwise a reader could scan unbounded
    // empty chunks to reach one rollup byte, breaking work-∝-rollup-bytes.
    if chunk_rollup == 0 || chunk_rollup > MAX_ROLLUP_CHUNK_LEN {
        return false;
    }
    if chunk_encoded == 0 || chunk_encoded > MAX_COMPRESSED_CHUNK_LEN {
        return false;
    }
    // The running sum must never overrun the declared rollup length.
    if covered.saturating_add(chunk_rollup as usize) > rollup_len {
        return false;
    }
    // A raw chunk carries its rollup bytes verbatim.
    if codec == CODEC_RAW_ESCAPE && chunk_encoded != chunk_rollup {
        return false;
    }
    true
}

/// Decode the chunk stream `payload` (the bytes after the fixed header, or the
/// not-yet-decoded suffix when extending incrementally) of a codec-`codec` envelope,
/// producing at most `max_rollup` rollup bytes.
///
/// `classify_and_decode` passes the header's full `rollup_len` (wholesale decode);
/// native `advance` passes the *remaining* budget (`rollup_len − already_decoded`) over
/// `&accumulator[consumed..]` to decode only the newly-authenticated chunks. Because each
/// chunk is an independent LZ4 block, concatenating incremental decodes is byte-for-byte
/// identical to one wholesale decode.
///
/// Returns `(rollup, consumed, clean)`:
/// * `rollup` — bytes from the complete chunks decoded,
/// * `consumed` — payload bytes consumed (whole chunks only),
/// * `clean` — `false` iff a structural error was hit (cap violation, overrun,
///   raw length mismatch, LZ4 failure); `true` if the decode is well-formed,
///   even when merely incomplete (a partial trailing chunk left unconsumed).
///
/// Never panics: every read is bounds-checked, and each chunk decodes into a single reused
/// buffer sized to the (cap-validated) per-chunk maximum, so memory use is bounded.
pub(crate) fn decode_chunks(payload: &[u8], codec: u8, max_rollup: u32) -> (Vec<u8>, usize, bool) {
    let rollup_len = max_rollup as usize;
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut covered = 0usize;
    // Reused across chunks so the LZ4 arm doesn't allocate (and zero) a fresh buffer per
    // chunk. Sized to the per-chunk cap and sliced to each chunk's length; only the bytes a
    // chunk decodes into are ever read back. Matters most in the guest, where this decode runs.
    let mut scratch = [0u8; MAX_ROLLUP_CHUNK_LEN as usize];

    while covered < rollup_len {
        // Need the 4-byte framing for the next chunk.
        if pos + CHUNK_HEADER_LEN > payload.len() {
            break; // truncated framing: incomplete, not corrupt
        }
        let chunk_rollup = u16::from_le_bytes([payload[pos], payload[pos + 1]]);
        let chunk_encoded = u16::from_le_bytes([payload[pos + 2], payload[pos + 3]]);

        // Validate framing before touching the payload bytes.
        if !chunk_framing_valid(codec, chunk_rollup, chunk_encoded, covered, rollup_len) {
            return (out, pos, false);
        }

        let body_start = pos + CHUNK_HEADER_LEN;
        let body_end = body_start + chunk_encoded as usize;
        if body_end > payload.len() {
            break; // truncated chunk payload: incomplete, not corrupt
        }
        let body = &payload[body_start..body_end];

        match codec {
            CODEC_RAW_ESCAPE => {
                // `chunk_framing_valid` guarantees encoded == rollup for raw chunks.
                out.extend_from_slice(body);
            }
            CODEC_LZ4 => {
                let dst = &mut scratch[..chunk_rollup as usize];
                match lz4_flex::block::decompress_into(body, dst) {
                    // Output size is the framing-supplied known size; reject any
                    // chunk that does not decode to exactly that many bytes.
                    Ok(n) if n == chunk_rollup as usize => out.extend_from_slice(dst),
                    _ => return (out, pos, false),
                }
            }
            // Unreachable: the header parse rejects unknown codecs. Fail closed.
            _ => return (out, pos, false),
        }

        covered += chunk_rollup as usize;
        pos = body_end;
    }

    (out, pos, true)
}

/// Classify and decode a blob's authenticated DA bytes.
///
/// Pure and deterministic: depends only on `buf`. This is the single source of
/// truth that both native extraction and the guest verifier call.
pub(crate) fn classify_and_decode(buf: &[u8]) -> EnvelopeState {
    if !has_magic_prefix(buf) {
        return EnvelopeState::Legacy;
    }
    let header = match parse_header(buf) {
        Ok(header) => header,
        Err(_) => return EnvelopeState::Malformed,
    };
    let (rollup, chunk_consumed, clean) =
        decode_chunks(&buf[ENVELOPE_HEADER_LEN..], header.codec, header.rollup_len);
    EnvelopeState::Envelope(DecodedEnvelope {
        header,
        rollup,
        consumed: ENVELOPE_HEADER_LEN + chunk_consumed,
        clean,
    })
}

/// Encode `rollup` as a chunked envelope with the given codec and chunk size.
///
/// `chunk_size` is the uncompressed bytes per chunk, passed in `1..=MAX_ROLLUP_CHUNK_LEN`, so
/// each chunk's rollup length satisfies the rollup cap by construction. A chunk's *encoded*
/// length must also satisfy [`MAX_COMPRESSED_CHUNK_LEN`]; a poorly-compressing chunk can exceed
/// it, which the canonical re-decode in `encode_for_submission` catches and falls back to raw.
#[cfg(feature = "native")]
pub(crate) fn encode_chunked(rollup: &[u8], codec: u8, chunk_size: usize) -> Vec<u8> {
    // Callers pass a config-validated size (`validate_compression_chunk_size` in
    // `config.rs`), so there is no silent clamp. The round-trip check in
    // `encode_for_submission` is the release backstop: an out-of-range size would
    // produce over-cap chunks that fail canonical re-decode and fall back to raw.
    debug_assert!(
        (1..=MAX_ROLLUP_CHUNK_LEN as usize).contains(&chunk_size),
        "compression_chunk_size {chunk_size} out of range 1..={MAX_ROLLUP_CHUNK_LEN}"
    );
    let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + rollup.len());
    out.extend_from_slice(&ENVELOPE_MAGIC);
    out.push(ENVELOPE_VERSION);
    out.push(codec);
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&(rollup.len() as u32).to_le_bytes());

    for chunk in rollup.chunks(chunk_size) {
        let encoded = match codec {
            CODEC_RAW_ESCAPE => chunk.to_vec(),
            // CODEC_LZ4: `compress` (no size prefix) — the framing carries the size.
            _ => lz4_flex::block::compress(chunk),
        };
        // chunk.len() <= chunk_size <= MAX_ROLLUP_CHUNK_LEN, and LZ4's worst-case output for
        // that (n + n/255 + 16) also fits u16 for any sane cap, so neither `as u16` truncates.
        // A chunk whose encoded length exceeds MAX_COMPRESSED_CHUNK_LEN is rejected by
        // `chunk_framing_valid` on re-decode, so `is_canonical_encoding_of` is the backstop
        // that falls back to verbatim.
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(encoded.len() as u16).to_le_bytes());
        out.extend_from_slice(&encoded);
    }
    out
}

/// True iff `frame` is a canonical envelope encoding of exactly `rollup`: it
/// decodes cleanly to `rollup`, the decoded length equals the declared
/// `rollup_len`, and the chunks consume the whole frame.
#[cfg(feature = "native")]
pub(crate) fn is_canonical_encoding_of(frame: &[u8], rollup: &[u8]) -> bool {
    match classify_and_decode(frame) {
        EnvelopeState::Envelope(d) => {
            d.is_canonical_for_frame_len(frame.len()) && d.rollup == rollup
        }
        EnvelopeState::Legacy | EnvelopeState::Malformed => false,
    }
}

/// Choose the bytes to post for a rollup batch payload.
///
/// * `compress == false` (config `Off`): post verbatim — today's behavior, which
///   preserves the upgrade window (a real Borsh batch never begins with the magic).
/// * `compress == true` (config `Lz4`): post a chunked LZ4 envelope iff it is
///   strictly smaller than the raw payload and round-trips canonically; otherwise
///   post the raw payload verbatim, escaping it as a raw-chunk envelope only when
///   it happens to begin with the magic (so a reader never mis-parses it).
#[cfg(feature = "native")]
pub(crate) fn encode_for_submission(rollup: &[u8], compress: bool, chunk_size: usize) -> Vec<u8> {
    if !compress {
        return rollup.to_vec();
    }

    let lz4 = encode_chunked(rollup, CODEC_LZ4, chunk_size);
    if lz4.len() < rollup.len() && is_canonical_encoding_of(&lz4, rollup) {
        return lz4;
    }

    if has_magic_prefix(rollup) {
        // Raw chunks carry rollup bytes verbatim (encoded == rollup), so they must honor the
        // compressed cap; clamp the chunk size so the escape is always canonical even when
        // `chunk_size` targets the larger rollup cap. Escaping is what stops a reader from
        // mis-parsing a magic-prefixed payload, so it must never silently fail.
        let escape_chunk_size = chunk_size.min(MAX_COMPRESSED_CHUNK_LEN as usize);
        let escaped = encode_chunked(rollup, CODEC_RAW_ESCAPE, escape_chunk_size);
        if is_canonical_encoding_of(&escaped, rollup) {
            return escaped;
        }
    }

    rollup.to_vec()
}

/// Decode DA bytes back to their rollup payload for a native read
/// (e.g. proof retrieval). Legacy and malformed-magic blobs pass through as their
/// DA bytes; a valid, canonical envelope yields its rollup payload.
#[cfg(feature = "native")]
pub(crate) fn decode_for_read(buf: &[u8]) -> Vec<u8> {
    match classify_and_decode(buf) {
        EnvelopeState::Envelope(d) if d.is_canonical_for_frame_len(buf.len()) => d.rollup,
        // Legacy / malformed-magic / non-canonical: DA bytes verbatim.
        _ => buf.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fixed header with the given fields (no chunks).
    fn header_bytes(version: u8, codec: u8, flags: u16, rollup_len: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(ENVELOPE_HEADER_LEN);
        v.extend_from_slice(&ENVELOPE_MAGIC);
        v.push(version);
        v.push(codec);
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&rollup_len.to_le_bytes());
        v
    }

    /// Append one chunk's framing + body to a frame.
    fn push_chunk(frame: &mut Vec<u8>, chunk_rollup: u16, body: &[u8]) {
        frame.extend_from_slice(&chunk_rollup.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u16).to_le_bytes());
        frame.extend_from_slice(body);
    }

    fn envelope(buf: &[u8]) -> DecodedEnvelope {
        match classify_and_decode(buf) {
            EnvelopeState::Envelope(d) => d,
            other => panic!("expected Envelope, got {other:?}"),
        }
    }

    #[test]
    fn legacy_when_no_magic() {
        let buf = b"\x00\x01 a borsh-ish payload long enough to clear the header length";
        assert!(!has_magic_prefix(buf));
        assert_eq!(classify_and_decode(buf), EnvelopeState::Legacy);
    }

    #[test]
    fn non_magic_frame_with_valid_header_bytes_is_legacy() {
        let mut buf = vec![0u8; ENVELOPE_HEADER_LEN];
        buf[OFFSET_VERSION] = ENVELOPE_VERSION;
        buf[OFFSET_CODEC] = CODEC_RAW_ESCAPE;
        assert_eq!(parse_header(&buf), Err(EnvelopeError::MissingMagic));
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Legacy);
    }

    #[test]
    fn rollup_len_is_little_endian() {
        // 1000 > 256, asymmetric LE bytes [0xE8, 0x03, 0x00, 0x00].
        let buf = header_bytes(ENVELOPE_VERSION, CODEC_LZ4, 0, 1000);
        assert_eq!(
            &buf[OFFSET_ROLLUP_LEN..OFFSET_ROLLUP_LEN + 4],
            &[0xE8, 0x03, 0x00, 0x00]
        );
        assert_eq!(parse_header(&buf).unwrap().rollup_len, 1000);
    }

    #[test]
    fn bad_version_is_malformed() {
        let buf = header_bytes(2, CODEC_RAW_ESCAPE, 0, 300);
        assert_eq!(parse_header(&buf), Err(EnvelopeError::BadVersion(2)));
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn nonzero_flags_is_malformed() {
        let buf = header_bytes(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0x0102, 300);
        assert_eq!(parse_header(&buf), Err(EnvelopeError::BadFlags(0x0102)));
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn unsupported_codec_is_malformed() {
        let buf = header_bytes(ENVELOPE_VERSION, 2, 0, 300); // 2 == reserved zstd
        assert_eq!(parse_header(&buf), Err(EnvelopeError::UnsupportedCodec(2)));
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn rollup_len_cap_violation_is_malformed() {
        let buf = header_bytes(ENVELOPE_VERSION, CODEC_LZ4, 0, MAX_ROLLUP_BLOB_LEN + 1);
        assert_eq!(
            parse_header(&buf),
            Err(EnvelopeError::RollupLenTooLarge {
                rollup_len: MAX_ROLLUP_BLOB_LEN + 1,
            })
        );
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn header_too_short_is_malformed() {
        let mut buf = ENVELOPE_MAGIC.to_vec();
        buf.extend_from_slice(&[1u8, 0u8]); // 18 bytes total
        assert_eq!(parse_header(&buf), Err(EnvelopeError::TooShort { len: 18 }));
        assert_eq!(classify_and_decode(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn lz4_round_trips_clean_and_canonical() {
        // Compressible, asymmetric content > 256 bytes. A small chunk size (one share) forces
        // several chunks so the test exercises multi-chunk tiling, not a single chunk.
        let rollup: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let frame = encode_chunked(&rollup, CODEC_LZ4, 482);
        let d = envelope(&frame);
        assert!(d.clean, "honest LZ4 frame must decode clean");
        assert_eq!(d.rollup, rollup);
        assert_eq!(d.rollup.len(), d.header.rollup_len as usize);
        assert_eq!(d.consumed, frame.len(), "canonical: chunks tile the frame");
        assert!(is_canonical_encoding_of(&frame, &rollup));
    }

    #[test]
    fn raw_escape_round_trips_clean_and_canonical() {
        // Raw chunks carry rollup bytes verbatim, so each must fit the compressed cap; a small
        // chunk size keeps every chunk within `MAX_COMPRESSED_CHUNK_LEN` and forces several chunks.
        let rollup: Vec<u8> = (0..1500u32).map(|i| (i % 97 + 1) as u8).collect();
        let frame = encode_chunked(&rollup, CODEC_RAW_ESCAPE, 482);
        let d = envelope(&frame);
        assert!(d.clean);
        assert_eq!(d.rollup, rollup);
        assert_eq!(d.consumed, frame.len());
        assert!(is_canonical_encoding_of(&frame, &rollup));
    }

    #[test]
    fn empty_rollup_is_clean_canonical_header_only() {
        let frame = encode_chunked(&[], CODEC_LZ4, MAX_ROLLUP_CHUNK_LEN as usize);
        assert_eq!(frame.len(), ENVELOPE_HEADER_LEN);
        let d = envelope(&frame);
        assert!(d.clean);
        assert!(d.rollup.is_empty());
        assert_eq!(d.consumed, ENVELOPE_HEADER_LEN);
    }

    #[test]
    fn partial_prefix_decodes_clean_but_incomplete() {
        // A strict prefix of a multi-chunk frame: clean, fewer rollup bytes,
        // consumed ends on a chunk boundary (the partial trailing chunk is dropped).
        let rollup: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let frame = encode_chunked(&rollup, CODEC_LZ4, 482);
        // Cut somewhere inside the chunk stream (past header, before the end).
        let cut = frame.len() - 7;
        let d = envelope(&frame[..cut]);
        assert!(
            d.clean,
            "a truncated trailing chunk is incomplete, not corrupt"
        );
        assert!(d.rollup.len() < rollup.len());
        assert_eq!(&d.rollup[..], &rollup[..d.rollup.len()], "decoded a prefix");
        assert!(d.consumed <= cut);
    }

    #[test]
    fn never_panics_on_any_truncation() {
        // Property: truncating an honest frame at every offset never panics.
        let rollup: Vec<u8> = (0..1300u32).map(|i| (i % 131) as u8).collect();
        let frame = encode_chunked(&rollup, CODEC_LZ4, 200);
        for cut in 0..=frame.len() {
            let _ = classify_and_decode(&frame[..cut]);
        }
    }

    #[test]
    fn corrupt_lz4_body_is_not_clean() {
        let rollup: Vec<u8> = (0..900u32).map(|i| (i % 200) as u8).collect();
        let mut frame = encode_chunked(&rollup, CODEC_LZ4, MAX_ROLLUP_CHUNK_LEN as usize);
        // Flip a byte in the first chunk's body (just past header + framing).
        let body0 = ENVELOPE_HEADER_LEN + CHUNK_HEADER_LEN;
        frame[body0] ^= 0xFF;
        let d = envelope(&frame);
        assert!(
            !d.clean,
            "a corrupt LZ4 chunk body must mark the decode unclean"
        );
    }

    #[test]
    fn zero_length_chunk_is_rejected() {
        let mut frame = header_bytes(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300);
        push_chunk(&mut frame, 0, &[]); // chunk_rollup == 0
        let d = envelope(&frame);
        assert!(!d.clean);
        assert!(d.rollup.is_empty());
    }

    #[test]
    fn over_cap_chunk_rollup_is_rejected() {
        // chunk_rollup declares one over the cap, before any body is read.
        assert!(!chunk_framing_valid(
            CODEC_LZ4,
            MAX_ROLLUP_CHUNK_LEN + 1,
            10,
            0,
            100_000
        ));
        assert!(!chunk_framing_valid(
            CODEC_LZ4,
            10,
            MAX_COMPRESSED_CHUNK_LEN + 1,
            0,
            100_000
        ));
    }

    #[test]
    fn running_sum_overrun_is_rejected() {
        // A chunk whose rollup bytes would push the running sum past rollup_len.
        assert!(!chunk_framing_valid(
            CODEC_LZ4, 100, 50, /*covered*/ 250, /*rollup*/ 300
        ));
        // Exactly reaching rollup_len is allowed.
        assert!(chunk_framing_valid(CODEC_LZ4, 50, 30, 250, 300));
    }

    #[test]
    fn raw_chunk_length_mismatch_is_rejected() {
        // Raw codec requires chunk_encoded == chunk_rollup.
        assert!(!chunk_framing_valid(CODEC_RAW_ESCAPE, 100, 99, 0, 1000));
        assert!(chunk_framing_valid(CODEC_RAW_ESCAPE, 100, 100, 0, 1000));
    }

    #[test]
    fn overrun_in_full_frame_is_not_clean() {
        // Header claims rollup_len 300, but a single chunk declares 400 rollup.
        let mut frame = header_bytes(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300);
        push_chunk(&mut frame, 400, &vec![7u8; 400]);
        let d = envelope(&frame);
        assert!(!d.clean, "running-sum overrun is a structural error");
    }

    #[test]
    fn encode_for_submission_off_is_verbatim() {
        let rollup: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(encode_for_submission(&rollup, false, 482), rollup);
    }

    #[test]
    fn encode_for_submission_compresses_when_smaller() {
        // Highly compressible payload -> an LZ4 envelope strictly smaller than raw.
        let rollup = vec![0xABu8; 4000];
        let da_payload = encode_for_submission(&rollup, true, 482);
        assert!(da_payload.len() < rollup.len(), "should compress");
        assert!(has_magic_prefix(&da_payload), "should be an envelope");
        assert_eq!(
            decode_for_read(&da_payload),
            rollup,
            "round-trips to rollup"
        );
    }

    /// High-entropy bytes (SplitMix64) that LZ4 cannot compress below raw.
    fn incompressible(len: usize) -> Vec<u8> {
        let mut z = 0x9e3779b97f4a7c15u64;
        (0..len)
            .map(|_| {
                z = z.wrapping_add(0x9e3779b97f4a7c15);
                let mut x = z;
                x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
                (x ^ (x >> 31)) as u8
            })
            .collect()
    }

    #[test]
    fn encode_for_submission_incompressible_is_verbatim() {
        // Magic-free high-entropy payload: LZ4 cannot beat raw -> DA verbatim.
        let rollup = incompressible(3000);
        assert!(!has_magic_prefix(&rollup));
        let da_payload = encode_for_submission(&rollup, true, 482);
        assert_eq!(
            da_payload, rollup,
            "incompressible -> verbatim, no envelope"
        );
    }

    #[test]
    fn encode_for_submission_escapes_magic_prefixed_payload() {
        // A magic-prefixed, incompressible payload must be wrapped so a reader does
        // not mis-parse it as a (foreign) envelope.
        let mut rollup = ENVELOPE_MAGIC.to_vec();
        rollup.extend(incompressible(2000));
        let da_payload = encode_for_submission(&rollup, true, 482);
        assert!(has_magic_prefix(&da_payload));
        assert_ne!(da_payload, rollup, "must be escaped, not posted raw");
        assert_eq!(decode_for_read(&da_payload), rollup, "escape round-trips");
        // And the escape is a raw-chunk envelope.
        assert_eq!(envelope(&da_payload).header.codec, CODEC_RAW_ESCAPE);
    }

    #[test]
    fn encode_for_submission_escapes_magic_prefix_with_oversized_chunk_size() {
        // With a chunk_size above the compressed cap, raw escape chunks would be over-cap and
        // the escape would fail, posting the magic-prefixed payload verbatim — which a reader
        // could mis-parse. The escape path clamps the chunk size to `MAX_COMPRESSED_CHUNK_LEN`,
        // so the payload is still escaped.
        let mut rollup = ENVELOPE_MAGIC.to_vec();
        rollup.extend(incompressible(2000));
        let da_payload = encode_for_submission(&rollup, true, MAX_ROLLUP_CHUNK_LEN as usize);
        assert!(has_magic_prefix(&da_payload));
        assert_ne!(da_payload, rollup, "must be escaped, not posted verbatim");
        assert_eq!(decode_for_read(&da_payload), rollup, "escape round-trips");
        assert_eq!(envelope(&da_payload).header.codec, CODEC_RAW_ESCAPE);
    }

    #[test]
    fn decode_for_read_passes_through_legacy() {
        let legacy = b"a legacy raw batch payload with no magic prefix at all..".to_vec();
        assert_eq!(decode_for_read(&legacy), legacy);
    }

    #[test]
    fn trailing_bytes_after_completion_are_left_unconsumed() {
        // A canonical frame followed by extra bytes: decode stops at rollup
        // completion (consumed < buf.len()), so the caller's
        // `consumed != compressed_total_len` check flags it as non-canonical.
        let rollup: Vec<u8> = (0..800u32).map(|i| (i % 200) as u8).collect();
        let mut frame = encode_chunked(&rollup, CODEC_LZ4, MAX_ROLLUP_CHUNK_LEN as usize);
        let canonical_len = frame.len();
        frame.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let d = envelope(&frame);
        assert!(d.clean);
        assert_eq!(d.rollup, rollup);
        assert_eq!(d.consumed, canonical_len, "stops at rollup completion");
        assert!(d.consumed < frame.len(), "trailing bytes left unconsumed");
        assert!(!is_canonical_encoding_of(&frame, &rollup));
    }

    #[test]
    fn lz4_round_trips_at_max_rollup_chunk_size() {
        // The decoupled caps let a chunk decode up to MAX_ROLLUP_CHUNK_LEN (16 KiB) as long as
        // its compressed form fits MAX_COMPRESSED_CHUNK_LEN. A highly compressible payload at
        // exactly the rollup cap exercises that >11x ratio and the decompression-bomb boundary.
        let chunk_len = MAX_ROLLUP_CHUNK_LEN as usize;
        let rollup: Vec<u8> = (0..chunk_len).map(|i| (i % 251) as u8).collect();
        let frame = encode_chunked(&rollup, CODEC_LZ4, chunk_len);

        // One chunk carrying the full rollup cap, compressed within the compressed cap.
        let chunk_rollup =
            u16::from_le_bytes([frame[ENVELOPE_HEADER_LEN], frame[ENVELOPE_HEADER_LEN + 1]]);
        let chunk_encoded = u16::from_le_bytes([
            frame[ENVELOPE_HEADER_LEN + 2],
            frame[ENVELOPE_HEADER_LEN + 3],
        ]);
        assert_eq!(
            chunk_rollup, MAX_ROLLUP_CHUNK_LEN,
            "single chunk at the rollup cap"
        );
        assert!(
            chunk_encoded <= MAX_COMPRESSED_CHUNK_LEN,
            "compressed chunk must fit the compressed cap: {chunk_encoded} > {MAX_COMPRESSED_CHUNK_LEN}"
        );

        let d = envelope(&frame);
        assert!(d.clean, "max-rollup-chunk LZ4 frame must decode clean");
        assert_eq!(d.rollup, rollup);
        assert_eq!(d.rollup.len(), d.header.rollup_len as usize);
        assert_eq!(
            d.consumed,
            frame.len(),
            "canonical: the chunk tiles the frame"
        );
        assert!(is_canonical_encoding_of(&frame, &rollup));
    }
}
