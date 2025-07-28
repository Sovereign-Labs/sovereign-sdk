use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SOV_EXPAND_PROC_MACROS");

    let constants_json_path = find_constants_manifest()?;
    if let Some(path) = constants_json_path {
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-env=CONSTANTS_MANIFEST_PATH={}", path.display());
    }

    Ok(())
}

pub fn find_constants_manifest() -> anyhow::Result<Option<PathBuf>> {
    // The flag check is a workaround to <https://github.com/dtlnay/trybuild/issues/231>.
    // Despite trybuild being a crate to build tests, it won't set the `test` flag. It isn't
    // setting the `trybuild` flag properly either.
    let filename = if cfg!(test) || env::var_os("SOV_TEST_MODE_CONST_MANIFEST").is_some() {
        // `constants.test.toml` would be better, but Taplo doesn't like it:
        // <https://github.com/tamasfe/taplo/issues/578>.
        "constants.testing.toml"
    } else {
        "constants.toml"
    };

    let dir_to_search = env::var_os("CONSTANTS_MANIFEST")
        .or_else(|| env::var_os("OUT_DIR"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Neither `CONSTANTS_MANIFEST` nor `OUT_DIR` are set; can't find `{}` file",
                filename
            )
        })?;

    // Iterate up the directory tree until we find a `constants.toml` file.
    let mut manifest_path = None;
    let mut dir: &Path = &dir_to_search;
    loop {
        let filepath = dir.join(filename);
        if filepath.is_file() {
            if manifest_path.is_some() {
                anyhow::bail!("Found multiple `{}` files in the parent directories of `{}`; remove all but one to avoid ambiguity", filename, dir_to_search.display());
            }

            match fs::read(&filepath) {
                Ok(_) => manifest_path = Some(filepath),
                Err(e) => anyhow::bail!(
                    "File `{}` found but not readable: {}",
                    filepath.display(),
                    e
                ),
            }
        }

        if let Some(parent) = dir.parent() {
            dir = parent;
        } else {
            break;
        }
    }

    Ok(manifest_path)
}
