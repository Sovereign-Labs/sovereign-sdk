use std::path::PathBuf;

use anyhow::{bail, Result};

pub(crate) const DEFAULT_CHAIN_ID: u64 = 7;
pub(crate) const DEFAULT_BLOCK_GAS_LIMIT: u64 = 30_000_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AllocStats {
    pub(crate) contract_accounts: usize,
    pub(crate) storage_slots: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct AdapterPaths {
    pub(crate) input_genesis: PathBuf,
    pub(crate) template_dir: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) chain_rlp_path: Option<PathBuf>,
}

impl AdapterPaths {
    pub(crate) fn usage(bin: &str) -> String {
        format!(
            "Usage: {bin} <geth_genesis.json> <template_genesis_dir> <output_dir> [chain_rlp_path]"
        )
    }

    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        if args.len() != 4 && args.len() != 5 {
            bail!("Invalid arguments");
        }

        Ok(Self {
            input_genesis: PathBuf::from(&args[1]),
            template_dir: PathBuf::from(&args[2]),
            output_dir: PathBuf::from(&args[3]),
            chain_rlp_path: args.get(4).map(PathBuf::from),
        })
    }
}
