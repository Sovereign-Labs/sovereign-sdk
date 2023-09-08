use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use toml::Table;

const MANIFEST_NAME: &str = "sovereign.toml";

/// Reads a `sovereign.toml` manifest file from the directory tree of the sov-modules-macros.
///
/// If the `RUSTFLAGS=--cfg procmacro2_semver_exempt` environment variable is set, then will read
/// the file from the directory of the provided span. Otherwise, will recurse from the manifest of
/// the `sov-modules-macros`. Warning: the latter approach might have edge cases as the compilation
/// of the `sov-modules-macros` might be performed under the
/// `$HOME.cargo/registry/src/index.crates.io-_/sov-modules-macros-_` folder.
///
/// Tracking issue: https://github.com/Sovereign-Labs/sovereign-sdk/issues/786
#[allow(dead_code)]
pub fn fetch_manifest_toml(span: Span) -> anyhow::Result<(PathBuf, Table)> {
    #[cfg(procmacro2_semver_exempt)]
    let initial_path = span
        .source_file()
        .path()
        .canonicalize()
        .map_err(|e| {
            anyhow::anyhow!("failed access base dir for sovereign manifest file from the span: {e}")
        })?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| {
            anyhow::anyhow!("Could not open the directory of the parent of the provided span")
        })?;

    let _ = span;

    #[cfg(not(procmacro2_semver_exempt))]
    let initial_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("failed access base dir for sovereign manifest file: {e}"))?;

    fetch_manifest_toml_from_path(initial_path)
}

fn fetch_manifest_toml_from_path<P>(initial_path: P) -> anyhow::Result<(PathBuf, Table)>
where
    P: AsRef<Path>,
{
    let path: PathBuf;
    let mut current_path = initial_path.as_ref();
    loop {
        if current_path.join(MANIFEST_NAME).exists() {
            path = current_path.join(MANIFEST_NAME);
            break;
        }

        current_path = current_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Could not find a parent {MANIFEST_NAME}"))?;
    }

    let manifest = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Could not read the parent `{}`: {e}", path.display()))?;

    let manifest = toml::from_str(&manifest)
        .map_err(|e| anyhow::anyhow!("Could not parse `{}`: {}", path.display(), e))?;

    Ok((path, manifest))
}

#[test]
fn fetch_manifest_works() {
    let path = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path).join("src").join("invalid");
    let (path, manifest) = fetch_manifest_toml_from_path(path).unwrap();

    let expected_path = env!("CARGO_MANIFEST_DIR");
    let expected_path = PathBuf::from(expected_path).join("sovereign.toml");
    let expected = fs::read_to_string(&expected_path).unwrap();
    let expected = toml::from_str(&expected).unwrap();

    assert_eq!(path, expected_path);
    assert_eq!(manifest, expected);
}
