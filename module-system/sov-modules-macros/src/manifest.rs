use std::path::PathBuf;
use std::{env, fs};

use toml::Table;

const MANIFEST_NAME: &str = "sovereign.toml";

/// Reads a `sovereign.toml` manifest file, recursing from the target directory that builds the
/// current implementation.
///
/// If the environment variable `SOVEREIGN_MANIFEST` is set, it will use that instead.
#[allow(dead_code)]
pub fn fetch_manifest_toml() -> anyhow::Result<Table> {
    let initial_path = match env::var("SOVEREIGN_MANIFEST") {
        Ok(p) => PathBuf::from(&p).canonicalize().map_err(|e| {
            anyhow::anyhow!("failed access base dir for sovereign manifest file `{p}`: {e}",)
        }),
        Err(_) => {
            // read the target directory set via build script since `OUT_DIR` is available only at build
            let initial_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .canonicalize()
                .map_err(|e| {
                    anyhow::anyhow!("failed access base dir for sovereign manifest file: {e}")
                })?
                .join("target-path");

            let initial_path = fs::read_to_string(&initial_path).map_err(|e| {
                anyhow::anyhow!("failed to read target path for sovereign manifest file: {e}")
            })?;

            PathBuf::from(initial_path.trim())
                .canonicalize()
                .map_err(|e| {
                    anyhow::anyhow!("failed access base dir for sovereign manifest file: {e}")
                })
        }
    }?;

    let path: PathBuf;
    let mut current_path = initial_path.as_path();
    loop {
        if current_path.join(MANIFEST_NAME).exists() {
            path = current_path.join(MANIFEST_NAME);
            break;
        }

        current_path = current_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Could not find a parent {}", MANIFEST_NAME))?;
    }

    let manifest = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Could not read the parent `{}`: {e}", path.display()))?;

    let manifest = toml::from_str(&manifest)
        .map_err(|e| anyhow::anyhow!("Could not parse `{}`: {}", path.display(), e))?;

    Ok(manifest)
}

#[test]
fn fetch_manifest_works() {
    let path = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(MANIFEST_NAME)
        .canonicalize()
        .unwrap();

    let expected = fs::read_to_string(path).unwrap();
    let expected = toml::from_str(&expected).unwrap();

    let manifest = fetch_manifest_toml().unwrap();
    assert_eq!(manifest, expected);
}
