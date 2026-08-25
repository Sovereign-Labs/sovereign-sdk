//! Tools for patching a rollup's chain identity without relinking its binaries.
//!
//! The patcher supports Linux native binaries and, with the `sp1` cargo feature, optional SP1
//! guest ELFs. Risc0 guest patching is not supported.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use object::{Object, ObjectSection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sov_chain_config::CHAIN_DATA_ELF_SECTION_NAME;
use sov_chain_config::{
    decode_chain_config_record, encode_chain_config_record, parse_chain_config_toml, ChainConfig,
    CHAIN_CONFIG_MAGIC, CHAIN_CONFIG_RECORD_SIZE, CHAIN_HASH_TEMPLATE_ELF_SECTION_NAME,
};
use sov_rollup_interface::common::HexHash;
use sov_universal_wallet::schema::{ChainData, ChainHashTemplate};

/// Inputs for producing a patched native artifact bundle with optional SP1 artifacts.
#[derive(Debug, Clone)]
pub struct PatchOptions {
    /// Linux native rollup executables to patch.
    pub native_binaries: Vec<PathBuf>,
    /// SP1 inner guest ELF to patch and recommit, or `None` for MockZkvm/native-only use.
    pub sp1_inner_elf: Option<PathBuf>,
    /// SP1 aggregation guest ELF, supplied together with `sp1_inner_elf`.
    pub sp1_outer_elf: Option<PathBuf>,
    /// `constants.toml` supplying replacement `CHAIN_ID`, `CHAIN_NAME`, and
    /// `CHAIN_HASH_OVERRIDES`; all other values are ignored.
    pub constants_toml: PathBuf,
    /// Destination directory, which must not already exist.
    pub output_dir: PathBuf,
}

/// Machine-readable description of a completed patch operation.
#[derive(Debug, Clone, Serialize)]
pub struct PatchManifest {
    /// Manifest format version.
    pub format_version: u32,
    /// Resulting chain configuration and chain hash.
    pub chain: ChainManifest,
    /// All copied artifacts and their content hashes.
    pub artifacts: Vec<ArtifactManifest>,
    /// Code commitments calculated when SP1 ELFs were supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sp1: Option<Sp1Manifest>,
}

/// Chain identity written to the output artifacts.
#[derive(Debug, Clone, Serialize)]
pub struct ChainManifest {
    /// Numeric chain identifier.
    pub chain_id: u64,
    /// Human-readable chain name.
    pub chain_name: String,
    /// Dynamic chain hash derived from the embedded schema template.
    pub chain_hash: HexHash,
    /// Resulting chain-hash override schedule.
    pub chain_hash_overrides: Vec<ChainHashOverrideManifest>,
}

/// One preserved historical chain hash.
#[derive(Debug, Clone, Serialize)]
pub struct ChainHashOverrideManifest {
    /// Inclusive first rollup height.
    pub start_height: u64,
    /// Exclusive last rollup height.
    pub end_height: u64,
    /// Historical chain hash.
    pub chain_hash: HexHash,
    /// Extra heights for which both adjacent hashes are accepted.
    pub grace_period: u64,
}

/// Description of one output binary.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactManifest {
    /// Artifact role in the proving pipeline.
    pub role: ArtifactRole,
    /// Source path.
    pub input: PathBuf,
    /// Path relative to the output directory.
    pub output: PathBuf,
    /// Byte offset of the patched chain-config record, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_config_offset: Option<u64>,
    /// Whether the chain-config record was changed.
    pub patched: bool,
    /// SHA-256 before patching.
    pub sha256_before: HexHash,
    /// SHA-256 in the output bundle.
    pub sha256_after: HexHash,
}

/// Role of an artifact in the bundle.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    /// Native rollup executable.
    Native,
    /// SP1 rollup-execution guest.
    Sp1Inner,
    /// SP1 aggregation guest.
    Sp1Outer,
}

/// SP1 commitments calculated from the output artifacts.
#[derive(Debug, Clone, Serialize)]
pub struct Sp1Manifest {
    /// Commitment to the patched inner ELF.
    pub inner_code_commitment: Sp1CommitmentManifest,
    /// Commitment to the unchanged outer ELF.
    pub outer_code_commitment: Sp1CommitmentManifest,
    /// The aggregation guest is not chain-specific and is intentionally not patched.
    pub outer_patched: bool,
}

/// Both representations of an SP1 method ID.
#[derive(Debug, Clone, Serialize)]
pub struct Sp1CommitmentManifest {
    /// Native SP1 `[u32; 8]` representation.
    pub words: [u32; 8],
    /// Canonical 32-byte big-endian hash.
    pub hash: HexHash,
}

struct InputArtifact {
    input: PathBuf,
    output: PathBuf,
    role: ArtifactRole,
    bytes: Vec<u8>,
    mode: Option<u32>,
}

struct PatchTarget {
    artifact: InputArtifact,
    config_offset: usize,
    config: ChainConfig,
    template: ChainHashTemplate,
}

struct OutputArtifact {
    artifact: InputArtifact,
    sha256_before: HexHash,
    config_offset: Option<usize>,
    patched: bool,
}

/// Finds the one valid fixed-size chain-config record in an artifact.
pub fn find_chain_config_record(bytes: &[u8]) -> Result<(usize, ChainConfig)> {
    let file = parse_elf(bytes)?;
    let (section_offset, section) = elf_section(&file, bytes, CHAIN_DATA_ELF_SECTION_NAME)?;
    find_record_in_section(section_offset, section)
}

fn find_record_in_section(section_offset: usize, section: &[u8]) -> Result<(usize, ChainConfig)> {
    let mut valid = Vec::new();
    let mut found_magic = Vec::new();
    let mut invalid = Vec::new();
    for relative_offset in magic_offsets(section, &CHAIN_CONFIG_MAGIC) {
        let offset = section_offset + relative_offset;
        found_magic.push(offset);
        let Some(candidate) =
            section.get(relative_offset..relative_offset + CHAIN_CONFIG_RECORD_SIZE)
        else {
            invalid.push(format!("offset {offset}: truncated record"));
            continue;
        };
        match decode_chain_config_record(candidate) {
            Ok(record) => valid.push((offset, record)),
            Err(error) => invalid.push(format!("offset {offset}: {error}")),
        }
    }
    match valid.len() {
        // False-positive magic bytes are harmless once exactly one fully validated record exists.
        1 => Ok(valid.pop().unwrap()),
        0 if found_magic.is_empty() => bail!("artifact contains no chain-config record"),
        0 => bail!(
            "artifact contains chain-config magic, but no valid record: {}",
            invalid.join("; ")
        ),
        count => bail!("artifact contains {count} valid chain-config records"),
    }
}

/// Finds and decodes the immutable Borsh chain-hash template in an artifact.
pub fn find_chain_hash_template(bytes: &[u8]) -> Result<ChainHashTemplate> {
    let file = parse_elf(bytes)?;
    let (_, section) = elf_section(&file, bytes, CHAIN_HASH_TEMPLATE_ELF_SECTION_NAME)?;
    ChainHashTemplate::from_borsh_bytes(section).context("invalid chain-hash template section")
}

fn parse_elf(bytes: &[u8]) -> Result<object::File<'_>> {
    let file = object::File::parse(bytes).context("artifact is not a valid object file")?;
    ensure!(
        file.format() == object::BinaryFormat::Elf,
        "artifact is not an ELF binary"
    );
    Ok(file)
}

fn elf_section<'a>(
    file: &object::File<'a>,
    bytes: &'a [u8],
    section_name: &str,
) -> Result<(usize, &'a [u8])> {
    let section = file
        .section_by_name(section_name)
        .with_context(|| format!("artifact has no {section_name} section"))?;
    let (offset, size) = section
        .file_range()
        .with_context(|| format!("{section_name} has no file-backed contents"))?;
    let offset = usize::try_from(offset)
        .with_context(|| format!("{section_name} section offset does not fit usize"))?;
    let size = usize::try_from(size)
        .with_context(|| format!("{section_name} section size does not fit usize"))?;
    let end = offset
        .checked_add(size)
        .with_context(|| format!("{section_name} section range overflows usize"))?;
    let section = bytes
        .get(offset..end)
        .with_context(|| format!("{section_name} section lies outside the artifact"))?;
    Ok((offset, section))
}

/// Replaces exactly one validated chain-config record, preserving every other byte.
pub fn patch_chain_config_record(
    bytes: &[u8],
    replacement: &ChainConfig,
) -> Result<(Vec<u8>, usize)> {
    let encoded = encode_chain_config_record(replacement)?;
    let (offset, _) = find_chain_config_record(bytes)?;
    let mut output = bytes.to_vec();
    patch_record_in_place(&mut output, offset, replacement, &encoded)?;
    Ok((output, offset))
}

/// Overwrites the record at a previously located offset and verifies the write by decoding the
/// patched bytes back.
fn patch_record_in_place(
    bytes: &mut [u8],
    offset: usize,
    replacement: &ChainConfig,
    encoded: &[u8; CHAIN_CONFIG_RECORD_SIZE],
) -> Result<()> {
    let record = bytes
        .get_mut(offset..offset + CHAIN_CONFIG_RECORD_SIZE)
        .context("chain-config record lies outside the artifact")?;
    record.copy_from_slice(encoded);
    let verified = decode_chain_config_record(record)?;
    ensure!(
        &verified == replacement,
        "patched record failed semantic verification"
    );
    Ok(())
}

/// Parses `CHAIN_ID`, `CHAIN_NAME`, and `CHAIN_HASH_OVERRIDES` from a full
/// `constants.toml` file. No other values are read or patched.
pub fn parse_constants_toml(bytes: &[u8]) -> Result<ChainConfig> {
    let text = std::str::from_utf8(bytes).context("constants.toml is not valid UTF-8")?;
    parse_chain_config_toml(text).map_err(anyhow::Error::msg)
}

/// Patches copied native artifacts and, when supplied, SP1 artifacts, then writes a bundle.
pub fn build_bundle(options: &PatchOptions) -> Result<PatchManifest> {
    ensure_linux_host()?;
    #[cfg(not(feature = "sp1"))]
    ensure!(
        options.sp1_inner_elf.is_none() && options.sp1_outer_elf.is_none(),
        "SP1 ELFs were supplied, but this sov-chain-config build does not include SP1 support \
         (enable the `sp1` cargo feature)"
    );
    ensure!(
        !options.native_binaries.is_empty(),
        "at least one native binary is required"
    );
    ensure!(
        !options.output_dir.exists(),
        "output directory {} already exists",
        options.output_dir.display()
    );
    ensure!(
        options.sp1_inner_elf.is_some() == options.sp1_outer_elf.is_some(),
        "SP1 inner and outer ELFs must be supplied together"
    );
    let constants_bytes = fs::read(&options.constants_toml).with_context(|| {
        format!(
            "failed to read constants file {}",
            options.constants_toml.display()
        )
    })?;
    let chain_config = parse_constants_toml(&constants_bytes)?;

    let mut output_names = BTreeSet::new();
    let mut targets = Vec::new();
    for input in &options.native_binaries {
        let filename = input
            .file_name()
            .context("native binary path has no filename")?;
        let output = PathBuf::from("native").join(filename);
        ensure!(
            output_names.insert(output.clone()),
            "two native binaries would produce the same output path {}",
            output.display()
        );
        targets.push(load_patch_target(input, output, ArtifactRole::Native)?);
    }
    let outer = match (&options.sp1_inner_elf, &options.sp1_outer_elf) {
        (Some(inner), Some(outer)) => {
            targets.push(load_patch_target(
                inner,
                PathBuf::from("sp1/inner.elf"),
                ArtifactRole::Sp1Inner,
            )?);
            Some(load_artifact(
                outer,
                PathBuf::from("sp1/outer.elf"),
                ArtifactRole::Sp1Outer,
            )?)
        }
        (None, None) => None,
        _ => unreachable!("SP1 inputs were validated as a pair"),
    };

    let first = targets.first().unwrap();
    for target in targets.iter().skip(1) {
        ensure!(
            target.config == first.config,
            "embedded chain configurations disagree between {} and {}",
            first.artifact.input.display(),
            target.artifact.input.display()
        );
        ensure!(
            target.template == first.template,
            "embedded chain-hash templates disagree between {} and {}",
            first.artifact.input.display(),
            target.artifact.input.display()
        );
    }
    let replacement = chain_config;
    let encoded_replacement = encode_chain_config_record(&replacement)?;

    let chain_hash = first.template.chain_hash(&ChainData {
        chain_id: replacement.chain_id,
        chain_name: replacement.chain_name.clone(),
    })?;

    let mut outputs = Vec::new();
    for mut target in targets {
        let sha256_before = sha256_hash(&target.artifact.bytes);
        let patched = target.artifact.bytes
            [target.config_offset..target.config_offset + CHAIN_CONFIG_RECORD_SIZE]
            != encoded_replacement;
        patch_record_in_place(
            &mut target.artifact.bytes,
            target.config_offset,
            &replacement,
            &encoded_replacement,
        )?;
        outputs.push(OutputArtifact {
            config_offset: Some(target.config_offset),
            artifact: target.artifact,
            sha256_before,
            patched,
        });
    }
    let sp1 = match outer {
        #[cfg(feature = "sp1")]
        Some(outer) => {
            let inner_bytes = outputs
                .iter()
                .find(|artifact| matches!(artifact.artifact.role, ArtifactRole::Sp1Inner))
                .map(|artifact| artifact.artifact.bytes.as_slice())
                .context("internal error: missing SP1 inner output")?;
            let inner_commitment = sov_sp1_adapter::host::code_commitment_from_elf(inner_bytes)
                .context("failed to derive commitment for patched SP1 inner ELF")?;
            let outer_commitment = sov_sp1_adapter::host::code_commitment_from_elf(&outer.bytes)
                .context("failed to derive commitment for SP1 outer ELF")?;
            let sha256_before = sha256_hash(&outer.bytes);
            outputs.push(OutputArtifact {
                artifact: outer,
                sha256_before,
                config_offset: None,
                patched: false,
            });
            Some(Sp1Manifest {
                inner_code_commitment: commitment_manifest(&inner_commitment),
                outer_code_commitment: commitment_manifest(&outer_commitment),
                outer_patched: false,
            })
        }
        #[cfg(not(feature = "sp1"))]
        Some(_) => unreachable!("SP1 inputs are rejected when the `sp1` feature is disabled"),
        None => None,
    };

    let artifact_manifests = outputs
        .iter()
        .map(|artifact| ArtifactManifest {
            role: artifact.artifact.role,
            input: artifact.artifact.input.clone(),
            output: artifact.artifact.output.clone(),
            chain_config_offset: artifact.config_offset.map(|offset| offset as u64),
            patched: artifact.patched,
            sha256_before: artifact.sha256_before,
            sha256_after: sha256_hash(&artifact.artifact.bytes),
        })
        .collect();
    let manifest = PatchManifest {
        format_version: 1,
        chain: ChainManifest {
            chain_id: replacement.chain_id,
            chain_name: replacement.chain_name.clone(),
            chain_hash: chain_hash.into(),
            chain_hash_overrides: replacement
                .chain_hash_overrides
                .iter()
                .map(|item| ChainHashOverrideManifest {
                    start_height: item.start_height,
                    end_height: item.end_height,
                    chain_hash: item.chain_hash.into(),
                    grace_period: item.grace_period,
                })
                .collect(),
        },
        artifacts: artifact_manifests,
        sp1,
    };

    fs::create_dir(&options.output_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            options.output_dir.display()
        )
    })?;
    for artifact in &outputs {
        write_output_file(
            &options.output_dir.join(&artifact.artifact.output),
            &artifact.artifact.bytes,
            artifact.artifact.mode,
        )?;
    }
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    write_output_file(
        &options.output_dir.join("manifest.json"),
        &manifest_bytes,
        None,
    )?;
    Ok(manifest)
}

fn load_patch_target(input: &Path, output: PathBuf, role: ArtifactRole) -> Result<PatchTarget> {
    let artifact = load_artifact(input, output, role)?;
    let (config_offset, config, template) = {
        let file = parse_elf(&artifact.bytes)
            .with_context(|| format!("failed to parse {}", input.display()))?;
        let (section_offset, section) =
            elf_section(&file, &artifact.bytes, CHAIN_DATA_ELF_SECTION_NAME)
                .with_context(|| format!("invalid chain config in {}", input.display()))?;
        let (config_offset, config) = find_record_in_section(section_offset, section)
            .with_context(|| format!("invalid chain config in {}", input.display()))?;
        let (_, template_section) =
            elf_section(&file, &artifact.bytes, CHAIN_HASH_TEMPLATE_ELF_SECTION_NAME)
                .with_context(|| format!("invalid chain-hash template in {}", input.display()))?;
        let template = ChainHashTemplate::from_borsh_bytes(template_section)
            .with_context(|| format!("invalid chain-hash template in {}", input.display()))?;
        (config_offset, config, template)
    };
    Ok(PatchTarget {
        artifact,
        config_offset,
        config,
        template,
    })
}

fn load_artifact(input: &Path, output: PathBuf, role: ArtifactRole) -> Result<InputArtifact> {
    let bytes =
        fs::read(input).with_context(|| format!("failed to read artifact {}", input.display()))?;
    parse_elf(&bytes).with_context(|| format!("{} is not a valid ELF binary", input.display()))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(
            fs::metadata(input)
                .with_context(|| format!("failed to stat artifact {}", input.display()))?
                .permissions()
                .mode(),
        )
    };
    #[cfg(not(unix))]
    let mode = None;
    Ok(InputArtifact {
        input: input.to_owned(),
        output,
        role,
        bytes,
        mode,
    })
}

fn magic_offsets<'a>(bytes: &'a [u8], magic: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    bytes
        .windows(magic.len())
        .enumerate()
        .filter_map(move |(offset, candidate)| (candidate == magic).then_some(offset))
}

#[cfg(feature = "sp1")]
fn commitment_manifest(commitment: &sov_sp1_adapter::SP1MethodId) -> Sp1CommitmentManifest {
    let hash = sov_rollup_interface::zk::CodeCommitmentTrait::to_hash(commitment)
        .as_bytes()
        .try_into()
        .expect("SP1 commitments always have the canonical 32-byte length");
    Sp1CommitmentManifest {
        words: commitment.0,
        hash: HexHash::new(hash),
    }
}

fn sha256_hash(bytes: &[u8]) -> HexHash {
    HexHash::new(Sha256::digest(bytes).into())
}

fn write_output_file(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_linux_host() -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn ensure_linux_host() -> Result<()> {
    bail!("sov-chain-config currently supports Linux hosts and Linux native binaries only")
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::write::Object as WritableObject;
    use object::{Architecture, BinaryFormat, Endianness, SectionKind};
    fn config_record() -> ChainConfig {
        ChainConfig {
            chain_id: 1,
            chain_name: "old-chain".to_owned(),
            chain_hash_overrides: Vec::new(),
        }
    }

    fn template() -> ChainHashTemplate {
        ChainHashTemplate::new([3; 32], vec![0, 1, 2, 3], [4; 32])
    }

    fn chain_data_section_bytes(config: &ChainConfig) -> Vec<u8> {
        let mut section = vec![9; 37];
        section.extend_from_slice(&encode_chain_config_record(config).unwrap());
        section.extend_from_slice(&[8; 29]);
        section
    }

    fn elf_with_sections(chain_data: &[u8], template_bytes: &[u8]) -> Vec<u8> {
        let mut elf =
            WritableObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
        let chain_data_id = elf.add_section(
            Vec::new(),
            CHAIN_DATA_ELF_SECTION_NAME.as_bytes().to_vec(),
            SectionKind::ReadOnlyData,
        );
        elf.append_section_data(chain_data_id, chain_data, 8);
        let template_id = elf.add_section(
            Vec::new(),
            CHAIN_HASH_TEMPLATE_ELF_SECTION_NAME.as_bytes().to_vec(),
            SectionKind::ReadOnlyData,
        );
        elf.append_section_data(template_id, template_bytes, 1);
        elf.write().unwrap()
    }

    fn synthetic_elf(config: &ChainConfig) -> Vec<u8> {
        elf_with_sections(
            &chain_data_section_bytes(config),
            &template().borsh_bytes().unwrap(),
        )
    }

    #[test]
    fn patch_changes_only_the_fixed_record() {
        let old = config_record();
        let input = synthetic_elf(&old);
        let (offset, _) = find_chain_config_record(&input).unwrap();
        let mut replacement = old;
        replacement.chain_id = 42;
        replacement.chain_name = "new-chain".to_owned();
        let (output, patched_offset) = patch_chain_config_record(&input, &replacement).unwrap();

        assert_eq!(patched_offset, offset);
        assert_eq!(&output[..offset], &input[..offset]);
        assert_eq!(
            &output[offset + CHAIN_CONFIG_RECORD_SIZE..],
            &input[offset + CHAIN_CONFIG_RECORD_SIZE..]
        );
        assert_eq!(find_chain_config_record(&output).unwrap().1, replacement);

        let (idempotent_output, idempotent_offset) =
            patch_chain_config_record(&output, &replacement).unwrap();
        assert_eq!(idempotent_offset, offset);
        assert_eq!(idempotent_output, output);
    }

    #[test]
    fn duplicate_valid_records_are_rejected() {
        let mut section = chain_data_section_bytes(&config_record());
        section.extend_from_slice(&encode_chain_config_record(&config_record()).unwrap());
        let input = elf_with_sections(&section, &template().borsh_bytes().unwrap());
        assert!(find_chain_config_record(&input)
            .unwrap_err()
            .to_string()
            .contains("2 valid"));
    }

    #[test]
    fn valid_record_outside_chain_data_section_is_ignored() {
        let mut input = synthetic_elf(&config_record());
        input.extend_from_slice(&encode_chain_config_record(&config_record()).unwrap());
        assert_eq!(find_chain_config_record(&input).unwrap().1, config_record());
    }

    #[test]
    fn malformed_template_is_rejected() {
        let mut template_bytes = template().borsh_bytes().unwrap();
        template_bytes.pop();
        let input = elf_with_sections(&chain_data_section_bytes(&config_record()), &template_bytes);
        assert!(find_chain_hash_template(&input).is_err());
    }

    #[test]
    fn constants_toml_supplies_all_patchable_values() {
        let hash = format!("0x{}", "ab".repeat(32));
        let constants = format!(
            r#"
[constants]
CHAIN_ID = 42
CHAIN_NAME = "new-chain"
CHAIN_HASH_OVERRIDES = [{{ start_height = 0, end_height = 12, chain_hash = "{hash}", grace_period = 3 }}]
UNRELATED_CONSTANT = 7

[gas]
another_unrelated_value = 1
"#
        );
        let parsed = parse_constants_toml(constants.as_bytes()).unwrap();
        assert_eq!(parsed.chain_id, 42);
        assert_eq!(parsed.chain_name, "new-chain");
        assert_eq!(parsed.chain_hash_overrides.len(), 1);
        assert_eq!(parsed.chain_hash_overrides[0].chain_hash, [0xab; 32]);
        assert_eq!(parsed.chain_hash_overrides[0].grace_period, 3);
    }

    #[test]
    fn native_only_bundle_supports_mock_zkvm() {
        let temp = tempfile::tempdir().unwrap();
        let input_path = temp.path().join("rollup");
        fs::write(&input_path, synthetic_elf(&config_record())).unwrap();
        let constants_path = temp.path().join("constants.toml");
        fs::write(
            &constants_path,
            "[constants]\nCHAIN_ID = 42\nCHAIN_NAME = \"mock-chain\"\nCHAIN_HASH_OVERRIDES = []\n",
        )
        .unwrap();
        let output_dir = temp.path().join("output");

        let manifest = build_bundle(&PatchOptions {
            native_binaries: vec![input_path],
            sp1_inner_elf: None,
            sp1_outer_elf: None,
            constants_toml: constants_path,
            output_dir: output_dir.clone(),
        })
        .unwrap();

        assert!(manifest.sp1.is_none());
        assert_eq!(manifest.artifacts.len(), 1);
        let output = fs::read(output_dir.join("native/rollup")).unwrap();
        let patched = find_chain_config_record(&output).unwrap().1;
        assert_eq!(patched.chain_id, 42);
        assert_eq!(patched.chain_name, "mock-chain");
    }
}
