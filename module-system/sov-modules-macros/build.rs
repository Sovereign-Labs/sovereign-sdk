use std::path::PathBuf;
use std::{env, fs, io};

fn main() -> io::Result<()> {
    // writes the target output dir into the manifest path so it can be later read for the
    // resolution of the sovereign.toml manifest file
    let target = env::var("OUT_DIR").map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let target = PathBuf::from(target).canonicalize()?.display().to_string();
    let manifest = env!("CARGO_MANIFEST_DIR");
    let manifest = PathBuf::from(manifest).canonicalize()?.join("target-path");
    fs::write(manifest, target)?;

    Ok(())
}
