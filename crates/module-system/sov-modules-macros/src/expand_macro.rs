//! Adapted from <https://github.com/drahnr/expander>, licensed under either MIT
//! or Apache-2.0.
//!
//! Copyright (c) Bernhard Schuster and other contributors.

use std::path::PathBuf;
use std::{env, fs};

use proc_macro::TokenStream;

/// Expand a proc-macro to file.
pub fn expand_to_file(input: TokenStream, filename: &str) -> Result<TokenStream, std::io::Error> {
    let out_dir = PathBuf::from(env!("OUT_DIR"));
    let dest = out_dir.join(filename);

    let input_str = input.to_string();
    // Sometimes proc-macros don't result in valid Rust code, so we
    // can't pretty print them. That's okay.
    let input_str = syn::parse_file(&input_str)
        .map(|f| prettier_please::unparse(&f))
        .unwrap_or(input_str);

    let dest = {
        let bytes = input_str.as_bytes();

        let hash = <blake2::Blake2s256 as blake2::Digest>::digest(bytes);
        // 12 bytes are more than enough to uniquely identify a proc-macro
        // expansion with enough confidence without making filenames
        // ridiculously long.
        let hex_suffix = hex::encode(&hash[..12]);

        PathBuf::from(dest.display().to_string() + "-" + &hex_suffix + ".rs")
    };

    eprintln!("expanding proc-macro to: {}", dest.display());
    fs::write(dest, input_str.as_bytes())?;

    Ok(input)
}
