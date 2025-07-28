use std::ops::Range;
use std::time::Duration;

use schemars::JsonSchema;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::Time;

use crate::storable::layer::StorableMockDaLayer;
use crate::{MockAddress, MockBlock, MockBlockHeader, MockHash};

/// Time in milliseconds to wait for the next block if it is not there yet.
/// How many times wait attempts are done depends on service configuration.
pub const WAIT_ATTEMPT_PAUSE: Duration = Duration::from_millis(10);
/// The max time for the requested block to be produced.
pub const DEFAULT_BLOCK_WAITING_TIME_MS: u64 = 120_000;

pub(crate) const GENESIS_HEADER: MockBlockHeader = MockBlockHeader {
    prev_hash: MockHash([0; 32]),
    hash: MockHash([1; 32]),
    height: 0,
    // 2023-01-01T00:00:00Z
    time: Time::from_millis(1672531200000),
};

pub(crate) const GENESIS_BLOCK: MockBlock = MockBlock {
    header: GENESIS_HEADER,
    batch_blobs: Vec::new(),
    proof_blobs: Vec::new(),
};

/// Configuration for block producing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockProducingConfig {
    /// Blocks are produced at fixed time intervals, regardless of whether
    /// there are transactions. This means empty blocks may be created.
    Periodic {
        /// The interval, in milliseconds, at which new blocks are produced.
        block_time_ms: u64,
    },

    /// A new block is produced only when a batch blob (but not a proof blob) is submitted.
    /// Each block contains exactly one batch blob and zero or more proof blobs.
    OnBatchSubmit {
        /// The maximum time [`sov_rollup_interface::node::da::DaService::get_block_at`] will wait for a block to become available.
        /// If this timeout elapses, an error is returned.
        /// If set to `None`, [`DEFAULT_BLOCK_WAITING_TIME_MS`] is used.
        block_wait_timeout_ms: Option<u64>,
    },

    /// A new block is produced when either a batch blob or a proof blob is submitted.
    /// Each block contains exactly one blob.
    OnAnySubmit {
        /// The maximum time [`sov_rollup_interface::node::da::DaService::get_block_at`] will wait for a block to become available.
        /// If this timeout elapses, an error is returned.
        /// If set to `None`, [`DEFAULT_BLOCK_WAITING_TIME_MS`] is used.
        block_wait_timeout_ms: Option<u64>,
    },
    /// Blocks are created manually, with no automatic production.
    Manual,
}

/// Defines the behavior of randomization applied to blobs or blocks.
///
/// This configurable behavior determines how blobs are processed and returned to the caller
/// during various stages of the block production process.
/// Randomization may involve reordering, shuffling, skipping, or altering the chain's length,
/// while preserving certain constraints such as finality.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RandomizationBehaviour {
    /// Blobs inside a single block are rearranged in a random order when read.
    /// This does not affect the boundary between blocks, meaning no blob will
    /// cross its original block's boundary.
    ///
    /// Notes:
    /// - Does not impact how new blocks are produced, and block hashes are not changed.
    /// - Finalized blocks may have their blobs reordered within this mode.
    /// - This does not change the stored order of blobs.
    /// - If randomization is disabled, blobs will be returned in their original order.
    /// - Order guaranteed to be deterministic for each block with the same randomizer configuration.
    OutOfOrderBlobs,
    /// Rewinds the chain to a specific height, chosen randomly between
    /// the most recently finalized block and the current head of the chain.
    ///
    /// This operation adjusts the chain height but maintains finalization constraints.
    Rewind,
    /// Combines blob shuffling with chain height adjustment:
    ///
    /// 1. All non-finalized blobs, including those being added to a new block,
    ///    are shuffled across all blobs that are part of the new chain state.
    /// 2. The chain height is adjusted (rewound or extended) within the constraints
    ///    of the finality window.
    ///
    /// **Constraints**:
    /// - Rewinding can only occur as far back as the finality window allows.
    /// - Extending is not possible if the finality window is already full.
    /// - Rewinding is not triggered if there is only one non-finalized block.
    /// - The specified percentage of blobs (`drop_percent`) is always respected.
    ShuffleAndResize {
        /// Percentage of blobs to be permanently skipped during this process.
        ///
        /// A value of `100` means all non-finalized blobs will be dropped.
        drop_percent: u8,
        /// Range of possible adjustments to the chain head height:
        /// - Negative values represent rewinding the chain length (moving backward in height).
        /// - Positive values represent extending the chain length (adding new blocks).
        /// - This adjustment is constrained by the finality window.
        ///
        /// The actual value is selected by [`crate::storable::layer::Randomizer`] from this range.
        adjust_head_height: Range<i32>,
    },
}

impl RandomizationBehaviour {
    /// Only shuffling without adjusting height of the rollup,
    pub fn only_shuffle(drop_percent: u8) -> Self {
        Self::ShuffleAndResize {
            drop_percent,
            adjust_head_height: 0..1,
        }
    }
}

/// Configuration for randomization applied.
///
/// This struct defines how randomization is performed, including the seed for the randomizer,
/// the timing of chain reorganization, and the specific randomization behavior applied.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct RandomizationConfig {
    /// Seed used by the randomizer to ensure deterministic but randomized behavior.
    pub seed: HexHash,
    /// The interval, in produced blocks, at which chain reorganization may occur.
    /// Applicable for all cases except [`RandomizationBehaviour::OutOfOrderBlobs`],
    /// which does not affect block production.
    ///
    /// For a range `m..n`:
    /// - A reorganization can occur at every `m`-th block produced after the last reorganization.
    /// - A reorganization will definitely occur at or before the `n`-th block produced since the last reorganization.
    ///
    /// Note:
    /// - The interval is counted starting from the height at which the last reorganization happened,
    ///   rather than the current state of the chain.
    /// - This allows the chain to progress consistently within the specified bounds between reorganizations.
    pub reorg_interval: Range<u32>,
    /// Defines the specific behavior of the randomizer during randomization.
    ///
    /// This determines how blobs or blocks are processed, including their ordering,
    /// shuffling, skipping, or potential adjustments affecting the chain.
    pub behaviour: RandomizationBehaviour,
}

/// The configuration for Mock Da.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct MockDaConfig {
    /// Connection string to the database for storing Da Data.
    ///   - "sqlite://demo_data/da.sqlite?mode=rwc"
    ///   - "sqlite::memory:"
    ///   - "postgresql://root:hunter2@aws.amazon.com/mock-da"
    pub connection_string: String,
    /// The address to use to "submit" blobs on the mock da layer.
    pub sender_address: MockAddress,
    /// Defines how many blocks progress to finalization.
    #[serde(default)]
    pub finalization_blocks: u32,
    /// How MockDaService should produce blocks.
    #[serde(default = "default_block_producing")]
    pub block_producing: BlockProducingConfig,
    /// Allow pointing to pre-existing [`StorableMockDaLayer`]
    #[serde(skip)]
    pub da_layer: Option<std::sync::Arc<tokio::sync::RwLock<StorableMockDaLayer>>>,
    /// If specified, [`StorableMockDaLayer`] will add randomization to non-finalized blocks.
    pub randomization: Option<RandomizationConfig>,
}

impl PartialEq for MockDaConfig {
    fn eq(&self, other: &Self) -> bool {
        let basic_eq = self.connection_string == other.connection_string
            && self.sender_address == other.sender_address
            && self.finalization_blocks == other.finalization_blocks
            && self.block_producing == other.block_producing
            && self.randomization == other.randomization;

        // Basic fields are not equal, no need to check da_layer field
        if !basic_eq {
            false
        } else {
            // We can only consider them Eq if `DaLayer` is None in both cases
            self.da_layer.is_none() && other.da_layer.is_none()
        }
    }
}

pub(crate) fn default_block_producing() -> BlockProducingConfig {
    BlockProducingConfig::OnBatchSubmit {
        block_wait_timeout_ms: Some(DEFAULT_BLOCK_WAITING_TIME_MS),
    }
}

impl MockDaConfig {
    /// Create [`MockDaConfig`] with instant finality.
    pub fn instant_with_sender(sender: MockAddress) -> Self {
        MockDaConfig {
            connection_string: Self::sqlite_in_memory(),
            sender_address: sender,
            finalization_blocks: 0,
            block_producing: default_block_producing(),
            da_layer: None,
            randomization: None,
        }
    }

    /// Connection string for in-memory SQLite.
    pub fn sqlite_in_memory() -> String {
        "sqlite::memory:".to_string()
    }

    /// Builds SQlite connection string and checks if a given directory exists.
    pub fn sqlite_in_dir(dir: impl AsRef<std::path::Path>) -> anyhow::Result<String> {
        let path = dir.as_ref();
        if !path.exists() {
            anyhow::bail!("Path {} does no exist", path.display());
        }
        let db_path = path.join("mock_da.sqlite");
        tracing::debug!(path = %db_path.display(), "Opening StorableMockDa");
        Ok(format!("sqlite://{}?mode=rwc", db_path.to_string_lossy()))
    }

    /// Instance of [`MockDaConfig`] that resembles Celestia DA. Batch production is periodic.
    pub fn celestia_like(connection_string: String, sender: MockAddress, seed: HexHash) -> Self {
        MockDaConfig {
            connection_string,
            sender_address: sender,
            finalization_blocks: 0,
            block_producing: BlockProducingConfig::Periodic {
                block_time_ms: 6_000,
            },
            da_layer: None,
            randomization: Some(RandomizationConfig {
                seed,
                // Not really applicable
                reorg_interval: Default::default(),
                // Just to spice things up a bit
                behaviour: RandomizationBehaviour::OutOfOrderBlobs,
            }),
        }
    }

    /// Instance of [`MockDaConfig`] that resembles Solana DA. Batch production is periodic.
    pub fn solana_like(connection_string: String, sender: MockAddress, seed: HexHash) -> Self {
        MockDaConfig {
            connection_string,
            sender_address: sender,
            finalization_blocks: 45,
            block_producing: BlockProducingConfig::Periodic { block_time_ms: 250 },
            da_layer: None,
            randomization: Some(RandomizationConfig {
                seed,
                reorg_interval: 10..20,
                behaviour: RandomizationBehaviour::ShuffleAndResize {
                    drop_percent: 5,
                    adjust_head_height: -10..10,
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_block_producing() {
        let config_s = r#"
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            finalization_blocks = 5
            [block_producing.periodic]
            block_time_ms = 1_000
        "#;
        let config = toml::from_str::<MockDaConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn manual_block_producing() {
        let config_s = r#"
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            [block_producing.manual]
        "#;
        let config = toml::from_str::<MockDaConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn with_randomization_shuffle_and_resize() {
        let config_s = r#"
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            finalization_blocks = 5
            [block_producing.periodic]
            block_time_ms = 1_000
            [randomization]
            seed = "0x0000000000000000000000000000000000000000000000000000000000000012"
            reorg_interval = [3, 5]
            [randomization.behaviour.shuffle_and_resize]
            drop_percent = 10
            adjust_head_height = [-3, 2]
        "#;
        let config = toml::from_str::<MockDaConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn with_randomization_rewind() {
        let config_s = r#"
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            finalization_blocks = 5
            [block_producing.periodic]
            block_time_ms = 1_000
            [randomization]
            seed = "0x0000000000000000000000000000000000000000000000000000000000000012"
            [randomization.reorg_interval]
            start = 3
            end = 5
            [randomization.behaviour.rewind]
        "#;
        let config = toml::from_str::<MockDaConfig>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }
}
