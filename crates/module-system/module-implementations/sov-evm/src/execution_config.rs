//! Provides configuration for native EVM execution.
use sov_modules_api::ExecutionInit;
use sov_modules_api::ModuleExecutionConfig;
use sov_modules_api::Spec;
use std::sync::OnceLock;
use std::sync::RwLock;

use crate::Evm;

/// Global storage for the EVM execution configuration.
///
/// Process-scoped: [`ModuleExecutionConfig::configure`] uses [`OnceLock::set`],
/// which is a one-shot per process. In production each node runs in its own
/// process so this is fine. In the test harness we rely on `cargo nextest`
/// (see `.config/nextest.toml`), which isolates every `#[tokio::test]` in a
/// fresh process — giving each test a virgin `OnceLock`. Invoking these
/// tests via `cargo test` (which shares a process per test binary) would
/// panic on the second call and is not supported.
pub static EVM_EXECUTION_CONFIG: OnceLock<RwLock<EvmExecutionConfig>> = OnceLock::new();

/// Configuration for native EVM execution.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmExecutionConfigContents {
    /// Whether to publish reverted transactions to the DA layer.
    #[serde(default)]
    pub preferred_sequencer_publish_reverted_txs: bool,
}

/// Configuration for native EVM execution.
#[derive(Clone, Debug)]
pub struct EvmExecutionConfig {
    /// The contents of the execution configuration.
    pub contents: EvmExecutionConfigContents,
    /// The location of the execution configuration file on disk. The file will get updated during execution.
    pub location: std::path::PathBuf,
}

impl<S: Spec> ExecutionInit for Evm<S> {
    type Config = EvmExecutionConfig;
    // Do nothing; the configure function handles everything we need.
    fn init(_config: &Self::Config) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
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
