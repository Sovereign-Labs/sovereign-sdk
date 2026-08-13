//! Patchable chain configuration: the fixed binary record format embedded in rollup
//! artifacts and the parsing of chain-identity values from `constants.toml`.

use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// ELF section containing patchable chain data.
pub const CHAIN_DATA_ELF_SECTION_NAME: &str = ".sov_chain_data";
/// Mach-O segment and section containing patchable chain data.
pub const CHAIN_DATA_MACHO_SECTION_NAME: &str = "__DATA,__sov_chain";
/// ELF section containing the immutable Borsh chain-hash template.
pub const CHAIN_HASH_TEMPLATE_ELF_SECTION_NAME: &str = ".sov_chain_template";
/// Mach-O segment and section containing the immutable Borsh chain-hash template.
pub const CHAIN_HASH_TEMPLATE_MACHO_SECTION_NAME: &str = "__DATA,__sov_tmpl";
/// Number of bytes before the payload in every embedded chain-data record.
pub const BINARY_SECTION_HEADER_LEN: usize = 64;

const VERSION_OFFSET: usize = 16;
const HEADER_LEN_OFFSET: usize = 20;
const PAYLOAD_LEN_OFFSET: usize = 24;
const PAYLOAD_CAPACITY_OFFSET: usize = 28;
const RECORD_DIGEST_OFFSET: usize = 32;

/// Describes one versioned, fixed-capacity record embedded in a binary section.
#[derive(Clone, Copy, Debug)]
pub struct BinarySectionSpec {
    magic: [u8; 16],
    version: u32,
    payload_capacity: usize,
    digest_domain: &'static [u8],
}

impl BinarySectionSpec {
    /// Creates a binary-section record specification.
    pub const fn new(
        magic: [u8; 16],
        version: u32,
        payload_capacity: usize,
        digest_domain: &'static [u8],
    ) -> Self {
        Self {
            magic,
            version,
            payload_capacity,
            digest_domain,
        }
    }

    /// Returns the total fixed record length for this specification.
    pub const fn record_len(self) -> usize {
        BINARY_SECTION_HEADER_LEN + self.payload_capacity
    }
}

/// A validated view of an embedded binary-section record.
#[derive(Clone, Copy, Debug)]
pub struct DecodedBinarySection<'a> {
    /// The unpadded encoded payload.
    pub payload: &'a [u8],
}

/// Error returned when encoding or decoding an embedded binary-section record.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BinarySectionError {
    /// The outer fixed-size record has the wrong length.
    #[error("invalid binary-section record length: expected {expected}, got {actual}")]
    InvalidRecordLength {
        /// Required record length.
        expected: usize,
        /// Supplied record length.
        actual: usize,
    },
    /// The record does not start with the expected magic value.
    #[error("invalid binary-section magic")]
    InvalidMagic,
    /// The record uses a different version from its specification.
    #[error("unsupported binary-section version {0}")]
    UnsupportedVersion(u32),
    /// The encoded header size differs from the shared fixed size.
    #[error("invalid binary-section header length: expected {expected}, got {actual}")]
    InvalidHeaderLength {
        /// Required header length.
        expected: usize,
        /// Encoded header length.
        actual: usize,
    },
    /// The encoded payload capacity differs from its specification.
    #[error("invalid binary-section payload capacity: expected {expected}, got {actual}")]
    InvalidPayloadCapacity {
        /// Required payload capacity.
        expected: usize,
        /// Encoded payload capacity.
        actual: usize,
    },
    /// The encoded payload exceeds the fixed body capacity.
    #[error("binary-section payload is {actual} bytes, exceeding the {capacity} byte capacity")]
    PayloadTooLarge {
        /// Encoded payload length.
        actual: usize,
        /// Fixed body capacity.
        capacity: usize,
    },
    /// The domain-separated digest does not match the header and unpadded payload.
    #[error("binary-section record digest mismatch")]
    DigestMismatch,
    /// Bytes after the payload are not canonical zero padding.
    #[error("binary-section record contains non-zero padding")]
    NonZeroPadding,
    /// A fixed-width header field was truncated.
    #[error("binary-section record contains a truncated header field")]
    TruncatedHeader,
}

/// Encodes a payload into a fixed-capacity binary record.
pub fn encode_binary_section(
    spec: BinarySectionSpec,
    payload: &[u8],
) -> Result<Vec<u8>, BinarySectionError> {
    if payload.len() > spec.payload_capacity {
        return Err(BinarySectionError::PayloadTooLarge {
            actual: payload.len(),
            capacity: spec.payload_capacity,
        });
    }

    let mut body = vec![0; spec.payload_capacity];
    body[..payload.len()].copy_from_slice(payload);
    let record_digest = binary_section_digest(spec, payload.len(), payload);

    let mut record = Vec::with_capacity(spec.record_len());
    record.extend_from_slice(&spec.magic);
    record.extend_from_slice(&spec.version.to_le_bytes());
    record.extend_from_slice(&(BINARY_SECTION_HEADER_LEN as u32).to_le_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&(spec.payload_capacity as u32).to_le_bytes());
    record.extend_from_slice(&record_digest);
    record.extend_from_slice(&body);
    debug_assert_eq!(record.len(), spec.record_len());
    Ok(record)
}

/// Validates a fixed-capacity binary record and returns its unpadded payload.
pub fn decode_binary_section<'a>(
    spec: BinarySectionSpec,
    record: &'a [u8],
) -> Result<DecodedBinarySection<'a>, BinarySectionError> {
    if record.len() != spec.record_len() {
        return Err(BinarySectionError::InvalidRecordLength {
            expected: spec.record_len(),
            actual: record.len(),
        });
    }
    if record[..spec.magic.len()] != spec.magic {
        return Err(BinarySectionError::InvalidMagic);
    }

    let version = read_u32(record, VERSION_OFFSET)?;
    if version != spec.version {
        return Err(BinarySectionError::UnsupportedVersion(version));
    }
    let header_len = read_u32(record, HEADER_LEN_OFFSET)? as usize;
    if header_len != BINARY_SECTION_HEADER_LEN {
        return Err(BinarySectionError::InvalidHeaderLength {
            expected: BINARY_SECTION_HEADER_LEN,
            actual: header_len,
        });
    }
    let payload_len = read_u32(record, PAYLOAD_LEN_OFFSET)? as usize;
    let payload_capacity = read_u32(record, PAYLOAD_CAPACITY_OFFSET)? as usize;
    if payload_capacity != spec.payload_capacity {
        return Err(BinarySectionError::InvalidPayloadCapacity {
            expected: spec.payload_capacity,
            actual: payload_capacity,
        });
    }
    if payload_len > payload_capacity {
        return Err(BinarySectionError::PayloadTooLarge {
            actual: payload_len,
            capacity: payload_capacity,
        });
    }

    let body = &record[header_len..];
    let expected_digest = binary_section_digest(spec, payload_len, &body[..payload_len]);
    if record[RECORD_DIGEST_OFFSET..header_len] != expected_digest {
        return Err(BinarySectionError::DigestMismatch);
    }
    if body[payload_len..].iter().any(|byte| *byte != 0) {
        return Err(BinarySectionError::NonZeroPadding);
    }

    Ok(DecodedBinarySection {
        payload: &body[..payload_len],
    })
}

fn binary_section_digest(spec: BinarySectionSpec, payload_len: usize, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(spec.digest_domain);
    hasher.update(spec.version.to_le_bytes());
    hasher.update((BINARY_SECTION_HEADER_LEN as u32).to_le_bytes());
    hasher.update((payload_len as u32).to_le_bytes());
    hasher.update((spec.payload_capacity as u32).to_le_bytes());
    hasher.update(payload);
    hasher.finalize().into()
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, BinarySectionError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(BinarySectionError::TruncatedHeader)?;
    Ok(u32::from_le_bytes(
        value.try_into().expect("integer has a fixed width"),
    ))
}

/// A chain hash override for a range of block heights.
///
/// The primary range is `[start_height, end_height)`. During the grace period after
/// `end_height`, both this hash and the next configured hash remain valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct ChainHashOverride {
    /// The start height, inclusive.
    pub start_height: u64,
    /// The end height, exclusive.
    pub end_height: u64,
    /// The chain hash to use for this range.
    pub chain_hash: [u8; 32],
    /// Number of blocks after `end_height` during which this hash is also accepted.
    #[cfg_attr(feature = "serde", serde(default))]
    pub grace_period: u64,
}

#[cfg(feature = "toml")]
#[derive(serde::Deserialize)]
struct ConstantsToml {
    constants: ChainConfigToml,
}

#[cfg(feature = "toml")]
#[derive(serde::Deserialize)]
struct ChainConfigToml {
    #[serde(rename = "CHAIN_ID")]
    chain_id: MaybeConst<u64>,
    #[serde(rename = "CHAIN_NAME")]
    chain_name: MaybeConst<String>,
    #[serde(rename = "CHAIN_HASH_OVERRIDES")]
    chain_hash_overrides: MaybeConst<Vec<ChainHashOverrideToml>>,
}

/// A TOML value that is written either bare (`CHAIN_ID = 4321`) or in the `{ const = ... }`
/// form the constants machinery uses to mark a value as usable in const contexts.
#[cfg(feature = "toml")]
#[derive(serde::Deserialize)]
#[serde(
    untagged,
    expecting = "a plain TOML value or a `{ const = ... }` table"
)]
enum MaybeConst<T> {
    Bare(T),
    Const {
        #[serde(rename = "const")]
        value: T,
    },
}

#[cfg(feature = "toml")]
impl<T> MaybeConst<T> {
    fn into_inner(self) -> T {
        match self {
            MaybeConst::Bare(value) | MaybeConst::Const { value } => value,
        }
    }
}

/// One `CHAIN_HASH_OVERRIDES` entry as written in `constants.toml`:
/// `{ start_height = <height>, end_height = <height>, chain_hash = "0x...", grace_period = <blocks> }`.
#[cfg(feature = "toml")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainHashOverrideToml {
    /// The start height, inclusive.
    pub start_height: u64,
    /// The end height, exclusive.
    pub end_height: u64,
    /// The chain hash as a hex string, with or without a `0x` prefix.
    pub chain_hash: String,
    /// Number of blocks after `end_height` during which this hash is still accepted.
    #[serde(default)]
    pub grace_period: u64,
}

#[cfg(feature = "toml")]
impl ChainHashOverrideToml {
    /// Decodes the hex chain hash and converts this entry into a [`ChainHashOverride`].
    pub fn resolve(&self) -> Result<ChainHashOverride, String> {
        let encoded = self
            .chain_hash
            .strip_prefix("0x")
            .unwrap_or(&self.chain_hash);
        let bytes = hex::decode(encoded).map_err(|error| format!("invalid chain hash: {error}"))?;
        let chain_hash = bytes.try_into().map_err(|bytes: Vec<u8>| {
            format!("chain hash must be exactly 32 bytes, got {}", bytes.len())
        })?;
        Ok(ChainHashOverride {
            start_height: self.start_height,
            end_height: self.end_height,
            chain_hash,
            grace_period: self.grace_period,
        })
    }
}

#[cfg(feature = "toml")]
fn resolve_chain_hash_overrides(
    items: &[ChainHashOverrideToml],
) -> Result<Vec<ChainHashOverride>, String> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.resolve()
                .map_err(|error| format!("chain hash override {index}: {error}"))
        })
        .collect()
}

/// Parses the patchable chain identity values from a complete `constants.toml` document.
///
/// Values outside `CHAIN_ID`, `CHAIN_NAME`, and `CHAIN_HASH_OVERRIDES` are ignored, but all
/// three keys must be present. Each value may use either the bare or the `{ const = ... }`
/// form.
#[cfg(feature = "toml")]
pub fn parse_chain_config_toml(input: &str) -> Result<ChainConfig, String> {
    let value: toml::Value = toml::from_str(input).map_err(|error| error.to_string())?;
    parse_chain_config_value(value)
}

/// Parses the patchable chain identity values from an already-parsed `constants.toml`
/// document. See [`parse_chain_config_toml`].
#[cfg(feature = "toml")]
pub fn parse_chain_config_value(value: toml::Value) -> Result<ChainConfig, String> {
    let input: ConstantsToml = value.try_into().map_err(|error| error.to_string())?;
    let config = ChainConfig {
        chain_id: input.constants.chain_id.into_inner(),
        chain_name: input.constants.chain_name.into_inner(),
        chain_hash_overrides: resolve_chain_hash_overrides(
            &input.constants.chain_hash_overrides.into_inner(),
        )?,
    };
    validate_chain_config(&config).map_err(|error| error.to_string())?;
    Ok(config)
}

/// Parses a bare TOML array of chain-hash overrides, such as the value of the
/// `SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES` test environment variable, applying the same
/// per-entry rules and schedule validation as [`parse_chain_config_toml`].
#[cfg(feature = "toml")]
pub fn parse_chain_hash_overrides_toml(input: &str) -> Result<Vec<ChainHashOverride>, String> {
    let deserializer = toml::de::ValueDeserializer::new(input);
    let items: Vec<ChainHashOverrideToml> =
        serde::Deserialize::deserialize(deserializer).map_err(|error| error.to_string())?;
    let overrides = resolve_chain_hash_overrides(&items)?;
    validate_chain_hash_overrides(&overrides).map_err(|error| error.to_string())?;
    Ok(overrides)
}

impl ChainHashOverride {
    /// Returns true if the height falls within this override's primary range.
    pub const fn contains(&self, height: u64) -> bool {
        height >= self.start_height && height < self.end_height
    }

    /// Returns true if the height falls within this override's grace period.
    pub const fn in_grace_period(&self, height: u64) -> bool {
        self.grace_period > 0
            && height >= self.end_height
            && height < self.end_height.saturating_add(self.grace_period)
    }
}

/// Magic prefix of a patchable chain-configuration record.
pub const CHAIN_CONFIG_MAGIC: [u8; 16] = *b"SOV_CHAIN_CONFIG";
/// Version of the patchable chain-configuration payload.
pub const CHAIN_CONFIG_VERSION: u32 = 1;
/// Capacity reserved for the Borsh chain-configuration payload.
pub const CHAIN_CONFIG_PAYLOAD_CAPACITY: usize = 8192;
/// Total fixed byte length of a patchable chain-configuration record.
pub const CHAIN_CONFIG_RECORD_SIZE: usize =
    BINARY_SECTION_HEADER_LEN + CHAIN_CONFIG_PAYLOAD_CAPACITY;
/// Maximum UTF-8 byte length of a chain name.
pub const CHAIN_CONFIG_MAX_NAME_LEN: usize = 256;
/// Maximum number of chain hash overrides in one record.
pub const CHAIN_CONFIG_MAX_OVERRIDES: usize = 64;
/// Domain separator used by the chain-configuration record digest.
pub const CHAIN_CONFIG_DIGEST_DOMAIN: &[u8] = b"SOV_CHAIN_CONFIG_V1";

const CHAIN_CONFIG_SPEC: BinarySectionSpec = BinarySectionSpec::new(
    CHAIN_CONFIG_MAGIC,
    CHAIN_CONFIG_VERSION,
    CHAIN_CONFIG_PAYLOAD_CAPACITY,
    CHAIN_CONFIG_DIGEST_DOMAIN,
);

/// Chain-specific values that can be patched into a compiled rollup artifact.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ChainConfig {
    /// Numeric identifier included in signed transactions.
    pub chain_id: u64,
    /// Human-readable chain name included in the transaction schema hash.
    pub chain_name: String,
    /// Historical schema hashes selected by rollup height.
    pub chain_hash_overrides: Vec<ChainHashOverride>,
}

/// Error returned when encoding, decoding, or validating a chain-configuration record.
#[derive(Debug, Error)]
pub enum ChainConfigError {
    /// The fixed binary-section envelope is invalid.
    #[error(transparent)]
    BinarySection(#[from] BinarySectionError),
    /// The Borsh payload could not be encoded or decoded.
    #[error("failed to encode or decode chain config payload: {0}")]
    Borsh(#[from] borsh::io::Error),
    /// The chain name exceeds [`CHAIN_CONFIG_MAX_NAME_LEN`].
    #[error("chain name is {actual} bytes, exceeding the {max} byte limit")]
    ChainNameTooLong {
        /// Actual UTF-8 byte length.
        actual: usize,
        /// Maximum permitted UTF-8 byte length.
        max: usize,
    },
    /// The override vector exceeds [`CHAIN_CONFIG_MAX_OVERRIDES`].
    #[error("chain config has {actual} overrides, exceeding the {max} entry limit")]
    TooManyOverrides {
        /// Actual number of overrides.
        actual: usize,
        /// Maximum permitted number of overrides.
        max: usize,
    },
    /// An override has an empty or reversed primary range.
    #[error("chain hash override {index} has an invalid range: {start_height}..{end_height}")]
    InvalidOverrideRange {
        /// Override index.
        index: usize,
        /// Inclusive range start.
        start_height: u64,
        /// Exclusive range end.
        end_height: u64,
    },
    /// The first override does not begin at rollup height zero.
    #[error("chain hash overrides must start at height 0, but start at {0}")]
    OverridesDoNotStartAtZero(u64),
    /// Adjacent override ranges are not contiguous.
    #[error(
        "chain hash overrides must be contiguous: previous range ends at {previous_end}, next range starts at {next_start}"
    )]
    NonContiguousOverrides {
        /// End of the previous range.
        previous_end: u64,
        /// Start of the next range.
        next_start: u64,
    },
}

/// Encodes a chain configuration into the shared fixed-size binary-section format.
pub fn encode_chain_config_record(
    config: &ChainConfig,
) -> Result<[u8; CHAIN_CONFIG_RECORD_SIZE], ChainConfigError> {
    validate_chain_config(config)?;
    let payload = borsh::to_vec(config)?;
    let encoded = encode_binary_section(CHAIN_CONFIG_SPEC, &payload)?;
    Ok(encoded
        .try_into()
        .expect("chain-config record length is fixed by its specification"))
}

/// Decodes and validates a fixed-size patchable chain-configuration record.
pub fn decode_chain_config_record(bytes: &[u8]) -> Result<ChainConfig, ChainConfigError> {
    let decoded = decode_binary_section(CHAIN_CONFIG_SPEC, bytes)?;
    let config = ChainConfig::try_from_slice(decoded.payload)?;
    validate_chain_config(&config)?;
    Ok(config)
}

fn validate_chain_config(config: &ChainConfig) -> Result<(), ChainConfigError> {
    if config.chain_name.len() > CHAIN_CONFIG_MAX_NAME_LEN {
        return Err(ChainConfigError::ChainNameTooLong {
            actual: config.chain_name.len(),
            max: CHAIN_CONFIG_MAX_NAME_LEN,
        });
    }
    if config.chain_hash_overrides.len() > CHAIN_CONFIG_MAX_OVERRIDES {
        return Err(ChainConfigError::TooManyOverrides {
            actual: config.chain_hash_overrides.len(),
            max: CHAIN_CONFIG_MAX_OVERRIDES,
        });
    }

    validate_chain_hash_overrides(&config.chain_hash_overrides)
}

/// Validates that chain-hash override ranges are non-empty, contiguous, and begin at zero.
pub fn validate_chain_hash_overrides(
    overrides: &[ChainHashOverride],
) -> Result<(), ChainConfigError> {
    for (index, hash_override) in overrides.iter().enumerate() {
        if hash_override.start_height >= hash_override.end_height {
            return Err(ChainConfigError::InvalidOverrideRange {
                index,
                start_height: hash_override.start_height,
                end_height: hash_override.end_height,
            });
        }
        if index == 0 && hash_override.start_height != 0 {
            return Err(ChainConfigError::OverridesDoNotStartAtZero(
                hash_override.start_height,
            ));
        }
        if let Some(previous) = index
            .checked_sub(1)
            .and_then(|previous| overrides.get(previous))
        {
            if previous.end_height != hash_override.start_height {
                return Err(ChainConfigError::NonContiguousOverrides {
                    previous_end: previous.end_height,
                    next_start: hash_override.start_height,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_override(start_height: u64, end_height: u64, byte: u8) -> ChainHashOverride {
        ChainHashOverride {
            start_height,
            end_height,
            chain_hash: [byte; 32],
            grace_period: 10,
        }
    }

    fn record() -> ChainConfig {
        ChainConfig {
            chain_id: 4321,
            chain_name: "A variable length chain name".to_owned(),
            chain_hash_overrides: vec![hash_override(0, 100, 1), hash_override(100, 200, 2)],
        }
    }

    #[test]
    fn fixed_record_roundtrip() {
        let expected = record();
        let encoded = encode_chain_config_record(&expected).unwrap();
        assert_eq!(encoded.len(), CHAIN_CONFIG_RECORD_SIZE);
        assert_eq!(decode_chain_config_record(&encoded).unwrap(), expected);
    }

    #[test]
    fn rejects_body_corruption() {
        let mut encoded = encode_chain_config_record(&record()).unwrap();
        encoded[BINARY_SECTION_HEADER_LEN + 8] ^= 1;
        let error = decode_chain_config_record(&encoded).unwrap_err();
        let ChainConfigError::BinarySection(inner) = error else {
            panic!("expected a binary-section error, got: {error}");
        };
        assert_eq!(inner, BinarySectionError::DigestMismatch);
    }

    #[test]
    fn rejects_nonzero_padding_with_a_valid_digest() {
        let payload = [1, 2, 3];
        let mut encoded = encode_binary_section(CHAIN_CONFIG_SPEC, &payload).unwrap();
        *encoded.last_mut().unwrap() = 1;
        let digest = binary_section_digest(CHAIN_CONFIG_SPEC, payload.len(), &payload);
        encoded[RECORD_DIGEST_OFFSET..BINARY_SECTION_HEADER_LEN].copy_from_slice(&digest);

        assert_eq!(
            decode_binary_section(CHAIN_CONFIG_SPEC, &encoded).unwrap_err(),
            BinarySectionError::NonZeroPadding
        );
    }

    #[test]
    fn rejects_invalid_override_ranges() {
        let mut record = record();
        record.chain_hash_overrides[1].start_height = 101;
        assert!(matches!(
            encode_chain_config_record(&record),
            Err(ChainConfigError::NonContiguousOverrides { .. })
        ));
    }

    #[cfg(feature = "toml")]
    #[test]
    fn toml_parser_rejects_missing_chain_hash_overrides() {
        let error =
            parse_chain_config_toml("[constants]\nCHAIN_ID = 4321\nCHAIN_NAME = \"TestChain\"\n")
                .unwrap_err();
        assert!(
            error.contains("CHAIN_HASH_OVERRIDES"),
            "missing CHAIN_HASH_OVERRIDES must be a parse error naming the key, got: {error}"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn toml_parser_accepts_const_value_form() {
        let config = parse_chain_config_toml(
            "[constants]\nCHAIN_ID = { const = 4321 }\nCHAIN_NAME = { const = \"TestChain\" }\nCHAIN_HASH_OVERRIDES = []\n",
        )
        .unwrap();
        assert_eq!(config.chain_id, 4321);
        assert_eq!(config.chain_name, "TestChain");
    }

    #[cfg(feature = "toml")]
    #[test]
    fn bare_override_array_parser_matches_embedded_rules() {
        let hash = "0x".to_owned() + &"12".repeat(32);
        let overrides = parse_chain_hash_overrides_toml(&format!(
            "[{{ start_height = 0, end_height = 10, chain_hash = \"{hash}\", grace_period = 5 }}]"
        ))
        .unwrap();
        assert_eq!(
            overrides,
            vec![ChainHashOverride {
                start_height: 0,
                end_height: 10,
                chain_hash: [0x12; 32],
                grace_period: 5,
            }]
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn bare_override_array_parser_validates_the_schedule() {
        let hash = "0x".to_owned() + &"12".repeat(32);
        let error = parse_chain_hash_overrides_toml(&format!(
            "[{{ start_height = 5, end_height = 10, chain_hash = \"{hash}\" }}]"
        ))
        .unwrap_err();
        assert!(
            error.contains("start at height 0"),
            "schedules not starting at zero must be rejected, got: {error}"
        );
    }
}
