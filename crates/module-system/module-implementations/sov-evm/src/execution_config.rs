//! Provides configuration of native execution; currently, this modules only includes configuration for pinning contract storage to RAM.
use alloy_primitives::Address;
use alloy_primitives::U256;
use sov_modules_api::ExecutionInit;
use sov_modules_api::ModuleExecutionConfig;
use sov_modules_api::Spec;
use sov_state::pinned_cache::BucketId;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::RwLock;

use crate::Evm;

/// Global storage for the EVM execution configuration.
pub static EVM_EXECUTION_CONFIG: OnceLock<RwLock<EvmExecutionConfig>> = OnceLock::new();

/// Configuration for EVM ram pinning.
///
/// Any addresses specified in `privileged_deployer_addresses` will automatically have their contracts pinned with a size limit of `default_bucket_size_limit`.
/// Any addresses specified in `known_contracts_and_limits` will be pinned with the specified size limit.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmExecutionConfigContents {
    /// Whether to publish reverted transactions to the DA layer.
    #[serde(default)]
    pub publish_reverted_txs: bool,
    /// The default size limit for any pinned bucket.
    #[serde(default = "default_bucket_size_limit")]
    pub default_bucket_size_limit: usize,
    /// Any addresses specified in `privileged_deployer_addresses` will automatically have their contracts pinned with a size limit of `default_bucket_size_limit`.
    #[serde(default)]
    pub privileged_deployer_addresses: Vec<Address>,
    /// The list of contracts to pin and their size limits.
    #[serde(default)]
    pub known_contracts_and_limits: BTreeMap<Address, usize>,
}

impl Default for EvmExecutionConfigContents {
    fn default() -> Self {
        Self {
            publish_reverted_txs: false,
            default_bucket_size_limit: default_bucket_size_limit(),
            privileged_deployer_addresses: vec![],
            known_contracts_and_limits: BTreeMap::new(),
        }
    }
}

/// Configuration for EVM ram pinning.
#[derive(Clone, Debug)]
pub struct EvmExecutionConfig {
    /// The contents of the execution configuration.
    pub contents: EvmExecutionConfigContents,
    /// The location of the execution configuration file on disk. The file will get updated during execution.
    pub location: std::path::PathBuf,
}

/// The default size limit for any new pinned bucket.
pub const fn default_bucket_size_limit() -> usize {
    100 * 1024 * 1024 // 100MB
}

impl<S: Spec> ExecutionInit for Evm<S> {
    type Config = EvmExecutionConfig;
    // Do nothing; the configure function handles everything we need.
    fn init(_config: &Self::Config) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}

impl<S: Spec> Evm<S> {
    pub(crate) fn get_bucket_id_for_address(&self, address: &Address) -> BucketId {
        let slot_key = self.account_storage.slot_key(&(address, &U256::ZERO));
        BucketId::from_slot_key(&slot_key, 21) // 21 bytes because the address is 20 bytes and the bcs prefixes with a length byte (always "20" (0x14))
    }

    /// Returns an iterator over the storage buckets to pin and their size limits.
    pub fn get_pinned_cache_buckets_and_limits(&self) -> Option<Vec<(BucketId, usize)>> {
        Some(
            EVM_EXECUTION_CONFIG
                .get()?
                .read()
                .expect("EVM Execution config RW lock is poisoned.")
                .contents
                .known_contracts_and_limits
                .iter()
                .map(|(address, limit)| {
                    let bucket_id = self.get_bucket_id_for_address(address);
                    (bucket_id, *limit)
                })
                .collect(),
        )
    }
}

impl ModuleExecutionConfig for EvmExecutionConfig {
    type Input = std::path::PathBuf;
    fn configure(input: &Self::Input) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !std::fs::exists(input)? {
            std::fs::write(
                input,
                serde_json::to_string_pretty(&EvmExecutionConfigContents::default())?,
            )?;
        }
        let file = std::fs::read(input)?;
        let config = serde_json::from_slice(&file)?;
        let config = EvmExecutionConfig {
            contents: config,
            location: input.clone(),
        };
        EVM_EXECUTION_CONFIG.set(RwLock::new(config)).map_err(|_| {
            "EVM Execution config already initialized. This is a bug, please report it."
        })?;
        Ok(())
    }
}
