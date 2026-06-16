//! Celestia compressed-blob envelope format (v1) parsing and classification.
//!
//! A rollup blob may be posted to Celestia either as today's raw payload or,
//! once compression lands, wrapped in a fixed-header *envelope*. This module
//! holds the pure, deterministic logic that recognizes and validates that
//! envelope from a blob's authenticated DA-physical bytes.
//!
//! Everything here is intentionally side-effect free and ungated: the guest
//! verifier and native extraction must classify identical bytes identically, so
//! the same compiled functions run in both. PR2 only introduces these helpers;
//! the live read/verify paths are wired up in PR3.
//!
//! ## Envelope format v1
//!
//! ```text
//! magic        [16] b"SOV_CELESTIA_CMP"
//! version      [1]  = 1
//! codec        [1]  0 = raw escape; 1 = LZ4 block; 2 reserved for zstd
//! flags        [2]  must be 0
//! logical_len  [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
//! payload      [..] codec-specific full-frame payload
//! ```

/// Envelope magic prefix. 128 bits, chosen so a standard Borsh batch/proof
/// payload beginning with these exact bytes would already be malformed in
/// practice.
pub(crate) const ENVELOPE_MAGIC: [u8; 16] = *b"SOV_CELESTIA_CMP";

/// The only recognized envelope format version.
pub(crate) const ENVELOPE_VERSION: u8 = 1;

/// Raw-escape codec: the payload is the logical bytes verbatim. Exists mainly to
/// escape payloads that naturally begin with [`ENVELOPE_MAGIC`].
pub(crate) const CODEC_RAW_ESCAPE: u8 = 0;

/// LZ4-block codec. The decode itself is introduced in PR3; PR2 only validates
/// that the header advertises a known codec.
pub(crate) const CODEC_LZ4: u8 = 1;

/// Fixed header size: magic[16] + version[1] + codec[1] + flags[2] + logical_len[4].
pub(crate) const ENVELOPE_HEADER_LEN: usize = 24;

/// Hard cap on the decoded logical payload length.
///
/// 64 MiB sits comfortably above any realistic rollup batch (the per-tx
/// `MAX_TX_SIZE` ceiling is 8 MiB) yet well below the ~126 MiB a Celestia data
/// square can physically hold, so a malicious `logical_len` cannot request an
/// unbounded allocation in PR3's decoder.
pub(crate) const MAX_LOGICAL_BLOB_LEN: u32 = 64 * 1024 * 1024;

/// Maximum allowed `logical_len / posted_payload_len`, checked before any decode
/// to reject decompression bombs cheaply.
pub(crate) const MAX_COMPRESSION_RATIO: u32 = 64;

// Header field offsets within a frame.
const OFFSET_VERSION: usize = 16;
const OFFSET_CODEC: usize = 17;
const OFFSET_FLAGS: usize = 18; // 2 bytes, u16 LE
const OFFSET_LOGICAL_LEN: usize = 20; // 4 bytes, u32 LE

/// The parsed and validated fixed envelope header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnvelopeHeader {
    /// Format version; always [`ENVELOPE_VERSION`] when present in a header.
    pub version: u8,
    /// Codec id; one of [`CODEC_RAW_ESCAPE`] or [`CODEC_LZ4`].
    pub codec: u8,
    /// Reserved flags; always 0 in v1.
    pub flags: u16,
    /// Declared decoded payload length.
    pub logical_len: u32,
}

/// Classification of a blob's authenticated DA-physical bytes.
///
/// Derived purely from the bytes, so it is identical on native and guest builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvelopeState {
    /// No magic prefix: a legacy raw blob, today's behavior.
    Legacy,
    /// Magic prefix present but the frame is not a valid v1 envelope. Treated as
    /// authenticated raw bytes (and therefore slashable) on the read path.
    Malformed,
    /// A valid v1 envelope header. PR2 stores only the header; PR3 attaches
    /// decoded logical bytes when it introduces decoding.
    Envelope(EnvelopeHeader),
}

/// Why a frame failed envelope validation.
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
    /// `logical_len / posted_payload_len` exceeds [`MAX_COMPRESSION_RATIO`].
    #[error("compression ratio too high: logical_len {logical_len} vs payload {payload_len}")]
    RatioTooHigh {
        /// Declared decoded length.
        logical_len: u32,
        /// Posted payload length (frame minus header).
        payload_len: usize,
    },
    /// A raw-escape (codec 0) frame whose payload length does not equal
    /// `logical_len`.
    #[error("raw-escape length mismatch: logical_len {logical_len} != payload {payload_len}")]
    LengthMismatch {
        /// Declared decoded length.
        logical_len: u32,
        /// Posted payload length (frame minus header).
        payload_len: usize,
    },
}

/// Returns true iff `buf` begins with the 16-byte envelope magic.
pub(crate) fn has_magic_prefix(buf: &[u8]) -> bool {
    buf.starts_with(&ENVELOPE_MAGIC)
}

/// Validate a full frame's header and length invariants.
///
/// The caller need not have checked the magic: non-magic frames return
/// [`EnvelopeError::MissingMagic`]. The classifier separately maps non-magic
/// blobs to [`EnvelopeState::Legacy`].
pub(crate) fn parse_envelope(buf: &[u8]) -> Result<EnvelopeHeader, EnvelopeError> {
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

    let payload_len = buf.len() - ENVELOPE_HEADER_LEN;
    // Multiply form avoids division (and the `payload_len == 0` divide-by-zero):
    // a zero payload only validates when `logical_len` is also zero.
    if logical_len as u64 > payload_len as u64 * MAX_COMPRESSION_RATIO as u64 {
        return Err(EnvelopeError::RatioTooHigh {
            logical_len,
            payload_len,
        });
    }

    // Raw escape must carry exactly the logical bytes.
    if codec == CODEC_RAW_ESCAPE && payload_len as u64 != logical_len as u64 {
        return Err(EnvelopeError::LengthMismatch {
            logical_len,
            payload_len,
        });
    }

    Ok(EnvelopeHeader {
        version,
        codec,
        flags,
        logical_len,
    })
}

/// Classify a blob's authenticated DA-physical bytes.
///
/// Pure and deterministic: depends only on `buf`. This is the single source of
/// truth that both native extraction and the guest verifier call in PR3.
pub(crate) fn classify(buf: &[u8]) -> EnvelopeState {
    if !has_magic_prefix(buf) {
        return EnvelopeState::Legacy;
    }
    match parse_envelope(buf) {
        Ok(header) => EnvelopeState::Envelope(header),
        Err(_) => EnvelopeState::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an envelope frame: magic + version + codec + flags(LE) + logical_len(LE) + payload.
    fn frame(version: u8, codec: u8, flags: u16, logical_len: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(ENVELOPE_HEADER_LEN + payload.len());
        v.extend_from_slice(&ENVELOPE_MAGIC);
        v.push(version);
        v.push(codec);
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&logical_len.to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn valid_header(codec: u8, logical_len: u32) -> EnvelopeState {
        EnvelopeState::Envelope(EnvelopeHeader {
            version: ENVELOPE_VERSION,
            codec,
            flags: 0,
            logical_len,
        })
    }

    #[test]
    fn legacy_when_no_magic() {
        let buf = b"\x00\x01 a borsh-ish payload long enough to clear the header length";
        assert!(!has_magic_prefix(buf));
        assert_eq!(classify(buf), EnvelopeState::Legacy);
    }

    #[test]
    fn parse_rejects_non_magic_frame_with_valid_header_bytes() {
        let mut buf = vec![0u8; ENVELOPE_HEADER_LEN];
        buf[OFFSET_VERSION] = ENVELOPE_VERSION;
        buf[OFFSET_CODEC] = CODEC_RAW_ESCAPE;
        assert_eq!(parse_envelope(&buf), Err(EnvelopeError::MissingMagic));
        assert_eq!(classify(&buf), EnvelopeState::Legacy);
    }

    #[test]
    fn logical_len_is_little_endian() {
        // 1000 > 256, asymmetric byte pattern: LE bytes are [0xE8, 0x03, 0x00, 0x00].
        let payload = vec![0xABu8; 1000];
        let buf = frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 1000, &payload);
        assert_eq!(
            &buf[OFFSET_LOGICAL_LEN..OFFSET_LOGICAL_LEN + 4],
            &[0xE8, 0x03, 0x00, 0x00]
        );
        assert_eq!(parse_envelope(&buf).unwrap().logical_len, 1000);
    }

    #[test]
    fn valid_raw_escape_classifies_as_envelope() {
        let payload = vec![0x5Au8; 300]; // > 256
        let buf = frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300, &payload);
        assert_eq!(classify(&buf), valid_header(CODEC_RAW_ESCAPE, 300));
    }

    #[test]
    fn raw_escape_length_mismatch_is_malformed() {
        let payload = vec![0x5Au8; 299]; // claims 300, has 299
        let buf = frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 300, &payload);
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::LengthMismatch {
                logical_len: 300,
                payload_len: 299,
            })
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn valid_lz4_header_classifies_without_decode() {
        // logical 4000, posted payload 500 -> 8x ratio, within cap; not decoded in PR2.
        let payload = vec![0x11u8; 500];
        let buf = frame(ENVELOPE_VERSION, CODEC_LZ4, 0, 4000, &payload);
        assert_eq!(classify(&buf), valid_header(CODEC_LZ4, 4000));
    }

    #[test]
    fn bad_version_is_malformed() {
        let payload = vec![0u8; 300];
        let buf = frame(2, CODEC_RAW_ESCAPE, 0, 300, &payload);
        assert_eq!(parse_envelope(&buf), Err(EnvelopeError::BadVersion(2)));
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn nonzero_flags_is_malformed() {
        let payload = vec![0u8; 300];
        let buf = frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0x0102, 300, &payload);
        assert_eq!(parse_envelope(&buf), Err(EnvelopeError::BadFlags(0x0102)));
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn unsupported_codec_is_malformed() {
        let payload = vec![0u8; 300];
        let buf = frame(ENVELOPE_VERSION, 2, 0, 300, &payload); // 2 == reserved zstd
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::UnsupportedCodec(2))
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn logical_len_cap_violation_is_malformed() {
        let payload = vec![0u8; 16];
        let buf = frame(
            ENVELOPE_VERSION,
            CODEC_LZ4,
            0,
            MAX_LOGICAL_BLOB_LEN + 1,
            &payload,
        );
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::LogicalLenTooLarge {
                logical_len: MAX_LOGICAL_BLOB_LEN + 1,
            })
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn ratio_cap_violation_is_malformed() {
        // payload 10 bytes, claim 10 * 64 + 1 logical -> just over the ratio cap.
        let payload = vec![0u8; 10];
        let claim = 10 * MAX_COMPRESSION_RATIO + 1;
        let buf = frame(ENVELOPE_VERSION, CODEC_LZ4, 0, claim, &payload);
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::RatioTooHigh {
                logical_len: claim,
                payload_len: 10,
            })
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn incomplete_evidence_header_too_short_is_malformed() {
        // Magic present but fewer than 24 header bytes total (18 here).
        let mut buf = ENVELOPE_MAGIC.to_vec();
        buf.extend_from_slice(&[1u8, 0u8]);
        assert!(has_magic_prefix(&buf));
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::TooShort { len: 18 })
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    #[test]
    fn zero_logical_zero_payload_is_valid_raw_escape() {
        // Divide-by-zero guard: empty payload, logical 0, codec 0 -> valid.
        let buf = frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 0, &[]);
        assert_eq!(classify(&buf), valid_header(CODEC_RAW_ESCAPE, 0));
    }

    #[test]
    fn zero_payload_nonzero_logical_is_malformed() {
        let buf = frame(ENVELOPE_VERSION, CODEC_LZ4, 0, 1, &[]);
        assert_eq!(
            parse_envelope(&buf),
            Err(EnvelopeError::RatioTooHigh {
                logical_len: 1,
                payload_len: 0,
            })
        );
        assert_eq!(classify(&buf), EnvelopeState::Malformed);
    }

    /// Documents the native/guest determinism guarantee: this body references no
    /// `native`-only symbol, so it is the same compiled mapping in both builds.
    #[test]
    fn classification_table_is_fixed() {
        let cases: [(Vec<u8>, EnvelopeState); 4] = [
            (
                b"plain legacy payload, no magic prefix at all....".to_vec(),
                EnvelopeState::Legacy,
            ),
            (
                frame(ENVELOPE_VERSION, CODEC_RAW_ESCAPE, 0, 512, &vec![7u8; 512]),
                valid_header(CODEC_RAW_ESCAPE, 512),
            ),
            (
                frame(ENVELOPE_VERSION, CODEC_LZ4, 0, 9000, &vec![3u8; 1000]),
                valid_header(CODEC_LZ4, 9000),
            ),
            (
                frame(2, CODEC_LZ4, 0, 9000, &vec![3u8; 1000]),
                EnvelopeState::Malformed,
            ),
        ];
        for (buf, expected) in cases {
            assert_eq!(classify(&buf), expected);
        }
    }
}
