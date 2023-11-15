use std::path::{Path, PathBuf};
use std::{env, fs, io};

use anyhow::{anyhow, Context};

fn resolve_manifest_path(out_dir: &Path) -> anyhow::Result<PathBuf> {
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
            // Start from the parent of out-dir to avoid a self-referential `cp` once the output file has been placed into `out_dir` for the first time
            let current_path = out_dir.parent().ok_or(anyhow!("out dir must have parent but {out_dir:?} had none"))?;
            let mut current_path = current_path.canonicalize().with_context(|| anyhow!("failed to canonicalize path to otput dirdir `{current_path:?}`"))?;
            loop {
                let path = current_path.join(manifest);
                if path.exists() && path.is_file() {
                    return Ok(path);
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
    let target_path_filename = std::env::var("TARGET_PATH_OVERRIDE").unwrap_or("target-path".to_string());
    let out_dir = env::var("OUT_DIR").map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let out_dir = PathBuf::from(out_dir);
    let out_dir = out_dir
        .canonicalize()
        .with_context(|| anyhow!("could not canonicalize out dir `{out_dir:?}`"))?;
    let manifest = env!("CARGO_MANIFEST_DIR");
    let target_path_record = PathBuf::from(manifest)
        .canonicalize()
        .with_context(|| anyhow!("could not canonicalize manifest dir path `{manifest:?}`"))?
        .join(target_path_filename);

    // Write the path "OUT_DIR" to a file in the manifest directory so that it's available to macros
    fs::write(target_path_record, out_dir.display().to_string())?;

    let input_manifest_path = resolve_manifest_path(&out_dir)?;
    let output_manifest_path = out_dir.join("constants.json");

    // Copy the constants.json file into out_dir
    let bytes_copied = fs::copy(&input_manifest_path, &output_manifest_path).with_context(|| anyhow!("could not copy manifest from {manifest:?} to output directory `{output_manifest_path:?}`"))?;
    if bytes_copied == 0 {
        return Err(anyhow!("Could not find valid `constants.json` Manifest file. The file at `{input_manifest_path:?}` was empty. You can set a different input file with the `CONSTANTS_MANIFEST` env var"));
    }
    // Tell cargo to rebuild if constants.json changes
    // We need to watch the output file in to handle the corner case where the user updates
    // their input manifest from `a/constants.json` to a pre-existing `b/constants.json` without updating any rust files.
    // Othewise, Cargo's timestamp based change detection will miss these changes.
    println!("cargo:rerun-if-changed={}", output_manifest_path.display());
    println!("cargo:rerun-if-changed={}", input_manifest_path.display());

    Ok(())
}
