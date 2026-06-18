//! Celestia compressed-blob envelope format (v1) parsing, decoding and encoding.
//!
//! A rollup blob may be posted to Celestia either as today's raw payload or,
//! once compression lands, wrapped in a fixed-header *envelope*. This module
//! holds the pure, deterministic logic that recognizes, validates and decodes
//! that envelope from a blob's authenticated DA-physical bytes.
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
//!   logical_len  [4]  u32 LE, total decoded length, <= MAX_LOGICAL_BLOB_LEN
//! then a sequence of independently-decodable chunks (the payload):
//!   chunk_logical_len [2]  u16 LE, 1..=MAX_LOGICAL_CHUNK_LEN
//!   chunk_encoded_len [2]  u16 LE, 1..=MAX_ENCODED_CHUNK_LEN
//!   chunk_payload     [chunk_encoded_len]
//! ```
//!
//! Chunks are decodable on their own, so reading the first N logical bytes only
//! authenticates and decodes the chunks covering them: verifier/guest work scales
//! with bytes consumed, not blob size.

/// Envelope magic prefix. 128 bits, chosen so a standard Borsh batch/proof
/// payload beginning with these exact bytes would already be malformed in
/// practice.
pub(crate) const ENVELOPE_MAGIC: [u8; 16] = *b"SOV_CELESTIA_CMP";

/// The only recognized envelope format version.
pub(crate) const ENVELOPE_VERSION: u8 = 1;

/// Raw-chunk codec: each chunk is the logical bytes verbatim
/// (`chunk_encoded_len == chunk_logical_len`). Exists mainly to escape payloads
/// that naturally begin with [`ENVELOPE_MAGIC`].
pub(crate) const CODEC_RAW_ESCAPE: u8 = 0;

/// LZ4-block codec. Each chunk is an independent LZ4 block decodable without any
/// later chunk, using the framing-supplied output size.
pub(crate) const CODEC_LZ4: u8 = 1;

/// Fixed header size: magic[16] + version[1] + codec[1] + flags[2] + logical_len[4].
pub(crate) const ENVELOPE_HEADER_LEN: usize = 24;

/// Per-chunk framing size: chunk_logical_len[2] + chunk_encoded_len[2].
pub(crate) const CHUNK_HEADER_LEN: usize = 4;

/// Hard cap on the decoded logical payload length.
///
/// 64 MiB sits comfortably above any realistic rollup batch (the per-tx
/// `MAX_TX_SIZE` ceiling is 8 MiB) yet well below the ~126 MiB a Celestia data
/// square can physically hold, so a malicious `logical_len` cannot request an
/// unbounded allocation.
pub(crate) const MAX_LOGICAL_BLOB_LEN: u32 = 64 * 1024 * 1024;

/// Maximum decoded length of a single chunk: one continuation sparse-share
/// payload (`CONTINUATION_SPARSE_SHARE_CONTENT_SIZE` = 482). Share-aligned, so it
/// is both the per-byte DoS bound and the partial-read granularity.
pub(crate) const MAX_LOGICAL_CHUNK_LEN: u16 = 482;

/// Maximum posted length of a single chunk: the LZ4 block worst-case output for
/// [`MAX_LOGICAL_CHUNK_LEN`] logical bytes (`n + n/255 + 16`). Bounds the bytes a
/// single-chunk read can be forced to authenticate.
pub(crate) const MAX_ENCODED_CHUNK_LEN: u16 =
    MAX_LOGICAL_CHUNK_LEN + MAX_LOGICAL_CHUNK_LEN / 255 + 16;

// Header field offsets within the fixed header.
const OFFSET_VERSION: usize = 16;
const OFFSET_CODEC: usize = 17;
const OFFSET_FLAGS: usize = 18; // 2 bytes, u16 LE
const OFFSET_LOGICAL_LEN: usize = 20; // 4 bytes, u32 LE

/// The parsed and validated fixed envelope header. `version` and `flags` are
/// validated against their only legal values during parsing, so only the fields
/// that drive decoding are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnvelopeHeader {
    /// Codec id; one of [`CODEC_RAW_ESCAPE`] or [`CODEC_LZ4`].
    pub codec: u8,
    /// Declared total decoded payload length.
    pub logical_len: u32,
}

/// The result of decoding the chunk stream of a valid-header envelope from a
/// (possibly partial) authenticated prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecodedEnvelope {
    /// The authenticated fixed header.
    pub header: EnvelopeHeader,
    /// Logical bytes produced by the complete chunks decoded so far.
    pub logical: Vec<u8>,
    /// DA-physical bytes consumed by the header plus the complete chunks decoded
    /// (a partial trailing chunk, or bytes past logical completion, are left
    /// unconsumed).
    pub consumed: usize,
    /// `false` iff the decoder hit a structural error (cap violation, running-sum
    /// overrun, raw-codec length mismatch, or LZ4 failure). A clean decode that is
    /// merely incomplete (a truncated trailing chunk on a partial read) stays
    /// `true`.
    pub clean: bool,
}

/// Classification of a blob's authenticated DA-physical bytes.
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
    /// The declared `logical_len` exceeds [`MAX_LOGICAL_BLOB_LEN`].
    #[error("logical_len {logical_len} exceeds cap")]
    LogicalLenTooLarge {
        /// The offending declared length.
        logical_len: u32,
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

    let logical_len = u32::from_le_bytes([
        buf[OFFSET_LOGICAL_LEN],
        buf[OFFSET_LOGICAL_LEN + 1],
        buf[OFFSET_LOGICAL_LEN + 2],
        buf[OFFSET_LOGICAL_LEN + 3],
    ]);
    if logical_len > MAX_LOGICAL_BLOB_LEN {
        return Err(EnvelopeError::LogicalLenTooLarge { logical_len });
    }

    Ok(EnvelopeHeader { codec, logical_len })
}

/// Validate a single chunk's framing *before* its (attacker-controlled-length)
/// payload is read or allocated. Shared by the decoder and the native
/// whole-chunk `advance`, so both bound the bytes a chunk read authenticates
/// identically.
///
/// `covered` is the running sum of `chunk_logical_len` decoded so far;
/// `logical_len` is the header's declared total.
pub(crate) fn chunk_framing_valid(
    codec: u8,
    chunk_logical: u16,
    chunk_encoded: u16,
    covered: usize,
    logical_len: usize,
) -> bool {
    // Zero-logical chunks are rejected: otherwise a reader could scan unbounded
    // empty chunks to reach one logical byte, breaking work-∝-logical-bytes.
    if chunk_logical == 0 || chunk_logical > MAX_LOGICAL_CHUNK_LEN {
        return false;
    }
    if chunk_encoded == 0 || chunk_encoded > MAX_ENCODED_CHUNK_LEN {
        return false;
    }
    // The running sum must never overrun the declared logical length.
    if covered.saturating_add(chunk_logical as usize) > logical_len {
        return false;
    }
    // A raw chunk carries its logical bytes verbatim.
    if codec == CODEC_RAW_ESCAPE && chunk_encoded != chunk_logical {
        return false;
    }
    true
}

/// Decode the chunk stream `payload` (everything after the fixed header) of a
/// codec-`codec` envelope declaring `logical_len` total logical bytes.
///
/// Returns `(logical, consumed, clean)`:
/// * `logical` — bytes from the complete chunks decoded,
/// * `consumed` — payload bytes consumed (whole chunks only),
/// * `clean` — `false` iff a structural error was hit (cap violation, overrun,
///   raw length mismatch, LZ4 failure); `true` if the decode is well-formed,
///   even when merely incomplete (a partial trailing chunk left unconsumed).
///
/// Never panics: every read is bounds-checked, and each chunk's output is sized
/// from the (cap-validated) framing, so allocation is bounded per chunk.
fn decode_chunks(payload: &[u8], codec: u8, logical_len: u32) -> (Vec<u8>, usize, bool) {
    let logical_len = logical_len as usize;
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut covered = 0usize;

    while covered < logical_len {
        // Need the 4-byte framing for the next chunk.
        if pos + CHUNK_HEADER_LEN > payload.len() {
            break; // truncated framing: incomplete, not corrupt
        }
        let chunk_logical = u16::from_le_bytes([payload[pos], payload[pos + 1]]);
        let chunk_encoded = u16::from_le_bytes([payload[pos + 2], payload[pos + 3]]);

        // Validate framing before touching the payload bytes.
        if !chunk_framing_valid(codec, chunk_logical, chunk_encoded, covered, logical_len) {
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
                // `chunk_framing_valid` guarantees encoded == logical for raw chunks.
                out.extend_from_slice(body);
            }
            CODEC_LZ4 => {
                let mut dst = vec![0u8; chunk_logical as usize];
                match lz4_flex::block::decompress_into(body, &mut dst) {
                    // Output size is the framing-supplied known size; reject any
                    // chunk that does not decode to exactly that many bytes.
                    Ok(n) if n == chunk_logical as usize => out.extend_from_slice(&dst),
                    _ => return (out, pos, false),
                }
            }
            // Unreachable: the header parse rejects unknown codecs. Fail closed.
            _ => return (out, pos, false),
        }

        covered += chunk_logical as usize;
        pos = body_end;
    }

    (out, pos, true)
}

/// Classify and decode a blob's authenticated DA-physical bytes.
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
    let (logical, chunk_consumed, clean) = decode_chunks(
        &buf[ENVELOPE_HEADER_LEN..],
        header.codec,
        header.logical_len,
    );
    EnvelopeState::Envelope(DecodedEnvelope {
        header,
        logical,
        consumed: ENVELOPE_HEADER_LEN + chunk_consumed,
        clean,
    })
}

/// Encode `logical` as a chunked envelope with the given codec and chunk size.
///
/// `chunk_size` is clamped to `1..=MAX_LOGICAL_CHUNK_LEN` so every produced chunk
/// passes the verifier's framing caps.
#[cfg(feature = "native")]
pub(crate) fn encode_chunked(logical: &[u8], codec: u8, chunk_size: usize) -> Vec<u8> {
    let chunk_size = chunk_size.clamp(1, MAX_LOGICAL_CHUNK_LEN as usize);
    let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + logical.len());
    out.extend_from_slice(&ENVELOPE_MAGIC);
    out.push(ENVELOPE_VERSION);
    out.push(codec);
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&(logical.len() as u32).to_le_bytes());

    for chunk in logical.chunks(chunk_size) {
        let encoded = match codec {
            CODEC_RAW_ESCAPE => chunk.to_vec(),
            // CODEC_LZ4: `compress` (no size prefix) — the framing carries the size.
            _ => lz4_flex::block::compress(chunk),
        };
        // chunk_size <= MAX_LOGICAL_CHUNK_LEN (482) and LZ4 worst-case output of 482
        // is MAX_ENCODED_CHUNK_LEN (499), so both lengths fit u16. The round-trip in
        // `is_canonical_encoding_of` is the backstop if that assumption ever breaks.
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(encoded.len() as u16).to_le_bytes());
        out.extend_from_slice(&encoded);
    }
    out
}

/// True iff `frame` is a canonical envelope encoding of exactly `logical`: it
/// decodes cleanly to `logical`, the decoded length equals the declared
/// `logical_len`, and the chunks consume the whole frame.
#[cfg(feature = "native")]
pub(crate) fn is_canonical_encoding_of(frame: &[u8], logical: &[u8]) -> bool {
    match classify_and_decode(frame) {
        EnvelopeState::Envelope(d) => {
            d.clean
                && d.consumed == frame.len()
                && d.logical.len() == d.header.logical_len as usize
                && d.logical == logical
        }
        EnvelopeState::Legacy | EnvelopeState::Malformed => false,
    }
}

/// Choose the bytes to post for a logical batch payload.
///
/// * `compress == false` (config `Off`): post verbatim — today's behavior, which
///   preserves the upgrade window (a real Borsh batch never begins with the magic).
/// * `compress == true` (config `Lz4`): post a chunked LZ4 envelope iff it is
///   strictly smaller than the raw payload and round-trips canonically; otherwise
///   post the raw payload verbatim, escaping it as a raw-chunk envelope only when
///   it happens to begin with the magic (so a reader never mis-parses it).
#[cfg(feature = "native")]
pub(crate) fn encode_for_submission(logical: &[u8], compress: bool, chunk_size: usize) -> Vec<u8> {
    if !compress {
        return logical.to_vec();
    }

    let lz4 = encode_chunked(logical, CODEC_LZ4, chunk_size);
    if lz4.len() < logical.len() && is_canonical_encoding_of(&lz4, logical) {
        return lz4;
    }

    if has_magic_prefix(logical) {
        let escaped = encode_chunked(logical, CODEC_RAW_ESCAPE, chunk_size);
        if is_canonical_encoding_of(&escaped, logical) {
            return escaped;
        }
    }

    logical.to_vec()
}

/// Decode posted bytes back to their logical payload for a native read
/// (e.g. proof retrieval). Legacy and malformed-magic blobs pass through as their
/// posted bytes; a valid, canonical envelope yields its logical payload.
#[cfg(feature = "native")]
pub(crate) fn decode_for_read(buf: &[u8]) -> Vec<u8> {
    match classify_and_decode(buf) {
        EnvelopeState::Envelope(d)
            if d.clean
                && d.consumed == buf.len()
                && d.logical.len() == d.header.logical_len as usize =>
        {
            d.logical
        }
        // Legacy / malformed-magic / non-canonical: posted bytes verbatim.
        _ => buf.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fixed header with the given fields (no chunks).
    fn header_bytes(version: u8, codec: u8, flags: u16, logical_len: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(ENVELOPE_HEADER_LEN);
        v.extend_from_slice(&ENVELOPE_MAGIC);
        v.push(version);
        v.push(codec);
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&logical_len.to_le_bytes());
        v
    }

    /// Append one chunk's framing + body to a frame.
    fn push_chunk(frame: &mut Vec<u8>, chunk_logical: u16, body: &[u8]) {
        frame.extend_from_slice(&chunk_logical.to_le_bytes());
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
    fn logical_len_is_little_endian() {
        // 1000 > 256, asymmetric LE bytes [0xE8, 0x03, 0x00, 0x00].
        let buf = header_bytes(ENVELOPE_VERSION, CODEC_LZ4, 0, 1000);
        assert_eq!(
            &buf[OFFSET_LOGICAL_LEN..OFFSET_LOGICAL_LEN + 4],
            &[0xE8, 0x03, 0x00, 0x00]
        );
        assert_eq!(parse_header(&buf).unwrap().logical_len, 1000);
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
    fn logical_len_cap_violation_is_malformed() {
        let buf = header_bytes(ENVELOPE_VERSION, CODEC_LZ4, 0, MAX_LOGICAL_BLOB_LEN + 1);
        assert_eq!(
            parse_header(&buf),
            Err(EnvelopeError::LogicalLenTooLarge {
                logical_len: MAX_LOGICAL_BLOB_LEN + 1,
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
        // Compressible, multi-chunk, asymmetric content > 256 bytes.
        let logical: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let frame = encode_chunked(&logical, CODEC_LZ4, MAX_LOGICAL_CHUNK_LEN as usize);
        let d = envelope(&frame);
        assert!(d.clean, "honest LZ4 frame must decode clean");
        assert_eq!(d.logical, logical);
        assert_eq!(d.logical.len(), d.header.logical_len as usize);
        assert_eq!(d.consumed, frame.len(), "canonical: chunks tile the frame");
        assert!(is_canonical_encoding_of(&frame, &logical));
    }

    #[test]
    fn raw_escape_round_trips_clean_and_canonical() {
        let logical: Vec<u8> = (0..1500u32).map(|i| (i % 97 + 1) as u8).collect();
        let frame = encode_chunked(&logical, CODEC_RAW_ESCAPE, MAX_LOGICAL_CHUNK_LEN as usize);
        let d = envelope(&frame);
        assert!(d.clean);
        assert_eq!(d.logical, logical);
        assert_eq!(d.consumed, frame.len());
        assert!(is_canonical_encoding_of(&frame, &logical));
    }

    #[test]
    fn empty_logical_is_clean_canonical_header_only() {
        let frame = encode_chunked(&[], CODEC_LZ4, MAX_LOGICAL_CHUNK_LEN as usize);
        assert_eq!(frame.len(), ENVELOPE_HEADER_LEN);
        let d = envelope(&frame);
        assert!(d.clean);
        assert!(d.logical.is_empty());
        assert_eq!(d.consumed, ENVELOPE_HEADER_LEN);
    }

    #[test]
    fn partial_prefix_decodes_clean_but_incomplete() {
        // A strict prefix of a multi-chunk frame: clean, fewer logical bytes,
        // consumed ends on a chunk boundary (the partial trailing chunk is dropped).
        let logical: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let frame = encode_chunked(&logical, CODEC_LZ4, MAX_LOGICAL_CHUNK_LEN as usize);
        // Cut somewhere inside the chunk stream (past header, before the end).
        let cut = frame.len() - 7;
        let d = envelope(&frame[..cut]);
        assert!(
            d.clean,
            "a truncated trailing chunk is incomplete, not corrupt"
        );
        assert!(d.logical.len() < logical.len());
        assert_eq!(
            &logical[..d.logical.len()],
            &d.logical[..],
            "decoded a prefix"
        );
        assert!(d.consumed <= cut);
    }

    #[test]
    fn never_panics_on_any_truncation() {
        // Property: truncating an honest frame at every offset never panics.
        let logical: Vec<u8> = (0..1300u32).map(|i| (i % 131) as u8).collect();
        let frame = encode_chunked(&logical, CODEC_LZ4, 200);
        for cut in 0..=frame.len() {
            let _ = classify_and_decode(&frame[..cut]);
        }
    }

    #[test]
    fn corrupt_lz4_body_is_not_clean() {
        let logical: Vec<u8> = (0..900u32).map(|i| (i % 200) as u8).collect();
        let mut frame = encode_chunked(&logical, CODEC_LZ4, MAX_LOGICAL_CHUNK_LEN as usize);
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
    fn zero_logical_chunk_is_rejected() {
        let mut frame = header_bytes(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300);
        push_chunk(&mut frame, 0, &[]); // chunk_logical == 0
        let d = envelope(&frame);
        assert!(!d.clean);
        assert!(d.logical.is_empty());
    }

    #[test]
    fn over_cap_chunk_logical_is_rejected() {
        // chunk_logical declares one over the cap, before any body is read.
        assert!(!chunk_framing_valid(
            CODEC_LZ4,
            MAX_LOGICAL_CHUNK_LEN + 1,
            10,
            0,
            100_000
        ));
        assert!(!chunk_framing_valid(
            CODEC_LZ4,
            10,
            MAX_ENCODED_CHUNK_LEN + 1,
            0,
            100_000
        ));
    }

    #[test]
    fn running_sum_overrun_is_rejected() {
        // A chunk whose logical bytes would push the running sum past logical_len.
        assert!(!chunk_framing_valid(
            CODEC_LZ4, 100, 50, /*covered*/ 250, /*logical*/ 300
        ));
        // Exactly reaching logical_len is allowed.
        assert!(chunk_framing_valid(CODEC_LZ4, 50, 30, 250, 300));
    }

    #[test]
    fn raw_chunk_length_mismatch_is_rejected() {
        // Raw codec requires chunk_encoded == chunk_logical.
        assert!(!chunk_framing_valid(CODEC_RAW_ESCAPE, 100, 99, 0, 1000));
        assert!(chunk_framing_valid(CODEC_RAW_ESCAPE, 100, 100, 0, 1000));
    }

    #[test]
    fn overrun_in_full_frame_is_not_clean() {
        // Header claims logical_len 300, but a single chunk declares 400 logical.
        let mut frame = header_bytes(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300);
        push_chunk(&mut frame, 400, &vec![7u8; 400]);
        let d = envelope(&frame);
        assert!(!d.clean, "running-sum overrun is a structural error");
    }

    #[test]
    fn encode_for_submission_off_is_verbatim() {
        let logical: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(encode_for_submission(&logical, false, 482), logical);
    }

    #[test]
    fn encode_for_submission_compresses_when_smaller() {
        // Highly compressible payload -> an LZ4 envelope strictly smaller than raw.
        let logical = vec![0xABu8; 4000];
        let posted = encode_for_submission(&logical, true, 482);
        assert!(posted.len() < logical.len(), "should compress");
        assert!(has_magic_prefix(&posted), "should be an envelope");
        assert_eq!(decode_for_read(&posted), logical, "round-trips to logical");
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
        // Magic-free high-entropy payload: LZ4 cannot beat raw -> posted verbatim.
        let logical = incompressible(3000);
        assert!(!has_magic_prefix(&logical));
        let posted = encode_for_submission(&logical, true, 482);
        assert_eq!(posted, logical, "incompressible -> verbatim, no envelope");
    }

    #[test]
    fn encode_for_submission_escapes_magic_prefixed_payload() {
        // A magic-prefixed, incompressible payload must be wrapped so a reader does
        // not mis-parse it as a (foreign) envelope.
        let mut logical = ENVELOPE_MAGIC.to_vec();
        logical.extend(incompressible(2000));
        let posted = encode_for_submission(&logical, true, 482);
        assert!(has_magic_prefix(&posted));
        assert_ne!(posted, logical, "must be escaped, not posted raw");
        assert_eq!(decode_for_read(&posted), logical, "escape round-trips");
        // And the escape is a raw-chunk envelope.
        assert_eq!(envelope(&posted).header.codec, CODEC_RAW_ESCAPE);
    }

    #[test]
    fn decode_for_read_passes_through_legacy() {
        let legacy = b"a legacy raw batch payload with no magic prefix at all..".to_vec();
        assert_eq!(decode_for_read(&legacy), legacy);
    }

    #[test]
    fn trailing_bytes_after_completion_are_left_unconsumed() {
        // A canonical frame followed by extra bytes: decode stops at logical
        // completion (consumed < buf.len()), so the caller's
        // `consumed != compressed_total_len` check flags it as non-canonical.
        let logical: Vec<u8> = (0..800u32).map(|i| (i % 200) as u8).collect();
        let mut frame = encode_chunked(&logical, CODEC_LZ4, MAX_LOGICAL_CHUNK_LEN as usize);
        let canonical_len = frame.len();
        frame.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let d = envelope(&frame);
        assert!(d.clean);
        assert_eq!(d.logical, logical);
        assert_eq!(d.consumed, canonical_len, "stops at logical completion");
        assert!(d.consumed < frame.len(), "trailing bytes left unconsumed");
        assert!(!is_canonical_encoding_of(&frame, &logical));
    }
}
