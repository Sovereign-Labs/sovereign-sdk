//! Minimal example CLI around the `sov-chain-config-patcher` library.
//!
//! The patcher operates on compiled artifacts and is not specific to any runtime, so this
//! binary works unchanged for any rollup; it lives in demo-rollup as the pattern to copy for
//! rollups that want to ship the tool from their own repository, alongside their own defaults.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use sov_chain_config_patcher::{build_bundle, PatchOptions};

/// Patch a rollup's chain identity and optionally refresh its SP1 commitments.
///
/// Produces a new bundle; source artifacts are never modified. Risc0 guest patching is not
/// supported. Only CHAIN_ID, CHAIN_NAME, and CHAIN_HASH_OVERRIDES are read from the constants
/// file; every other value is ignored.
#[derive(Debug, Parser)]
struct Cli {
    /// constants.toml supplying CHAIN_ID, CHAIN_NAME, and CHAIN_HASH_OVERRIDES only.
    /// All other values in the file are ignored and are not patched.
    #[arg(long)]
    constants: PathBuf,

    /// Linux native rollup executable to patch. Repeat for node/CLI variants.
    #[arg(long = "native", required = true)]
    native_binaries: Vec<PathBuf>,

    /// SP1 rollup-execution guest ELF to patch. Omit both SP1 arguments for MockZkvm.
    #[arg(long, requires = "sp1_outer_elf")]
    sp1_inner_elf: Option<PathBuf>,

    /// SP1 aggregation guest ELF to copy unchanged and recommit.
    #[arg(long, requires = "sp1_inner_elf")]
    sp1_outer_elf: Option<PathBuf>,

    /// New destination directory. It must not already exist.
    #[arg(long)]
    output_dir: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let manifest = build_bundle(&PatchOptions {
        native_binaries: cli.native_binaries,
        sp1_inner_elf: cli.sp1_inner_elf,
        sp1_outer_elf: cli.sp1_outer_elf,
        constants_toml: cli.constants,
        output_dir: cli.output_dir.clone(),
    })?;
    println!("Wrote patched bundle to {}", cli.output_dir.display());
    println!("Chain hash: {}", manifest.chain.chain_hash);
    if let Some(sp1) = manifest.sp1 {
        println!("SP1 inner commitment: {}", sp1.inner_code_commitment.hash);
        println!("SP1 outer commitment: {}", sp1.outer_code_commitment.hash);
    }
    Ok(())
}
