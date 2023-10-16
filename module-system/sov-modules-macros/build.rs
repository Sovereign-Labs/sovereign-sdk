use std::path::PathBuf;
use std::{env, fs, io};

use anyhow::{anyhow, Context};

fn resolve_manifest_path() -> anyhow::Result<PathBuf> {
    let manifest = "constants.json";
    match env::var("CONSTANTS_MANIFEST"){
        Ok(p) if p.is_empty() => {
            Err(anyhow!(
                "failed to read target path for sovereign manifest file: env var `CONSTANTS_MANIFEST` was set to the empty string"
            ))
        },
        Ok(p) => PathBuf::from(&p).canonicalize().map_err(|e| {
                anyhow!("failed to canonicalize path for sovereign manifest file from env var `{p}`: {e}")
        }),
        Err(_) => {
            let mut current_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            loop {
                if current_path.join(manifest).exists() {
                    return Ok(current_path.join(manifest));
                }

                current_path = current_path.parent().ok_or_else(|| {
                        anyhow!("Could not find a parent `{manifest}`")
                })?.to_path_buf();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    // writes the target output dir into the manifest path so it can be later read for the
    // resolution of the sovereign.toml manifest file
    let out_dir = env::var("OUT_DIR").map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let out_dir = PathBuf::from(out_dir);
    let out_dir = out_dir
        .canonicalize()
        .with_context(|| anyhow!("could not canonicalize out dir `{out_dir:?}`"))?;
    let manifest = env!("CARGO_MANIFEST_DIR");
    let target_path_record = PathBuf::from(manifest)
        .canonicalize()
        .with_context(|| anyhow!("could not canonicalize manifest dir path `{manifest:?}`"))?
        .join("target-path");

    // Write the path "OUT_DIR" to a file in the manifest directory so that it's available to macros
    fs::write(target_path_record, out_dir.display().to_string())?;

    let manifest_path = resolve_manifest_path()?;
    let output_manifest_path = out_dir.join("constants.json");

    // Copy the constants.json file into out_dir
    fs::copy(&manifest_path, &output_manifest_path).with_context(|| anyhow!("could not copy manifest from {manifest:?} to output directory `{output_manifest_path:?}`"))?;
    // Tell cargo to rebuild if constants.json changes
    println!("cargo:rerun-if-changed={}", output_manifest_path.display());
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    Ok(())
}
