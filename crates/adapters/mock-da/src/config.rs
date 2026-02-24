use std::ops::Range;
use std::time::Duration;

use schemars::JsonSchema;
use sha2::Digest;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::Time;

use crate::storable::layer::StorableMockDaLayer;
use crate::{MockAddress, MockBlock, MockBlockHeader, MockHash};

/// Time in milliseconds to wait for the next block if it is not there yet.
/// How many times wait attempts are done depends on service configuration.
pub const WAIT_ATTEMPT_PAUSE: Duration = Duration::from_millis(10);
/// The max time for the requested block to be produced.
pub const DEFAULT_BLOCK_WAITING_TIME_MS: u64 = 120_000;

/// How often we expect blocks to be produced, even if it is manual or on-batch submit.
/// It is based on expected time of node processing single block in debug build mode
pub(crate) const SENSIBLE_BLOCK_PULL_TIME: std::time::Duration =
    std::time::Duration::from_millis(200);

pub(crate) const GENESIS_HEADER: MockBlockHeader = MockBlockHeader {
    prev_hash: MockHash([255; 32]),
    // Unify with how MockBlockHeader::new or ::from_height are called
    hash: MockHash([
        0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ]),
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
    /// Makes `get_head_block_header` and `get_last_finalized_block_header` randomly
    /// return block headers below the actual finalized height.
    ///
    /// This simulates scenarios where the DA layer reports stale data,
    /// useful for testing rollup resilience to DA layer inconsistencies.
    ///
    /// Behavior:
    /// - Each call advances the internal RNG, returning a different height each time.
    /// - `get_last_finalized_block_header()` stores its result as a floor for head.
    /// - `get_head_block_header()` returns `max(computed_height, last_finalized_floor)`.
    /// - To guarantee `head >= finalized`, call `get_last_finalized_block_header()` first.
    ///
    /// Notes:
    /// - Does not affect actual block production or chain state.
    /// - Triggered probabilistically based on `reorg_interval` configuration.
    /// - Heights are deterministic given the same seed and call sequence.
    RewindBelowLastFinalized {
        /// Maximum number of blocks below finalized height to report.
        /// Random height is chosen between `max(0, finalized - max_depth)` and `finalized`.
        max_depth: u32,
    },
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

/// Configurable failure behavior for testing error handling in consumers of MockDa.
/// This allows tests to inject failures at specific points during DA operations.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureBehavior {
    /// No failures (default behavior).
    #[default]
    None,
    /// Fail `get_block_at` or `get_block_header_at` after N successful calls.
    /// The counter decrements on each call; when it reaches 0, failures may occur
    /// based on the configured probability.
    FailAfterNCalls {
        /// Number of successful calls remaining before failures may start.
        remaining: u64,
        /// Probability of failure (0-100). 100 = always fail, 0 = never fail.
        #[serde(default = "default_failure_probability")]
        failure_probability: u8,
    },
    /// Trigger a reorg (shuffle non-finalized blobs) when `get_block_at` or
    /// `get_block_header_at` is called for a specific height.
    /// After the reorg is triggered, subsequent calls proceed normally.
    ReorgDuringCall {
        /// The height that triggers the reorg.
        trigger_at_height: u64,
        /// Whether the reorg has already been triggered (runtime state, not serialized).
        #[serde(skip, default)]
        triggered: bool,
    },
    /// Add artificial delays to `get_block_at` or `get_block_header_at` after N calls.
    /// Useful for testing timeout handling and slow DA scenarios.
    DelayAfterNCalls {
        /// Number of calls before delays start.
        remaining: u64,
        /// Range of delay in milliseconds. A random value from this range is used.
        delay_range_ms: std::ops::Range<u64>,
    },
}

fn default_failure_probability() -> u8 {
    100
}

/// Result of checking failure behavior.
#[derive(Debug)]
pub enum CheckResult {
    /// No action needed, proceed normally.
    Ok,
    /// Add delay before proceeding.
    Delay(u64),
    /// Trigger a reorg (shuffle non-finalized blobs).
    TriggerReorg,
    /// Fail with error message.
    Fail(String),
}

/// Self-contained failure injection controller.
/// Holds behavior state and its own RNG for deterministic testing.
pub struct FailureInjector {
    behavior: FailureBehavior,
    rng: rand_chacha::ChaChaRng,
}

impl FailureInjector {
    /// Create injector with given behavior and seed.
    pub fn new(behavior: FailureBehavior, seed: u64) -> Self {
        use rand::SeedableRng;
        Self {
            behavior,
            rng: rand_chacha::ChaChaRng::seed_from_u64(seed),
        }
    }

    /// Create injector with no failures.
    pub fn none() -> Self {
        Self::new(FailureBehavior::None, 0)
    }

    /// Set new behavior, preserving RNG state.
    pub fn set_behavior(&mut self, behavior: FailureBehavior) {
        self.behavior = behavior;
    }

    /// Check failure behavior. Returns action to take based on current state.
    pub fn check(&mut self, height: u64) -> CheckResult {
        use rand::Rng;

        match &mut self.behavior {
            FailureBehavior::None => CheckResult::Ok,

            FailureBehavior::FailAfterNCalls {
                remaining,
                failure_probability,
            } => {
                if *remaining > 0 {
                    *remaining -= 1;
                    return CheckResult::Ok;
                }
                let roll: u8 = self.rng.gen_range(0..100);
                if roll < *failure_probability {
                    return CheckResult::Fail(format!(
                        "Injected failure (probability={failure_probability}%)"
                    ));
                }
                CheckResult::Ok
            }

            FailureBehavior::ReorgDuringCall {
                trigger_at_height,
                triggered,
            } => {
                if height == *trigger_at_height && !*triggered {
                    *triggered = true;
                    return CheckResult::TriggerReorg;
                }
                CheckResult::Ok
            }

            FailureBehavior::DelayAfterNCalls {
                remaining,
                delay_range_ms,
            } => {
                if *remaining > 0 {
                    *remaining -= 1;
                    return CheckResult::Ok;
                }
                CheckResult::Delay(self.rng.gen_range(delay_range_ms.clone()))
            }
        }
    }
}

/// Small, but more entropy seed, suitable for unit tests
pub fn seed_for_test(small_seed: u8) -> HexHash {
    let orig = [small_seed; 32];
    let mut hasher = sha2::Sha256::new();
    hasher.update(orig);
    let result = hasher.finalize();
    let mut hashed_seed = [0u8; 32];
    hashed_seed.copy_from_slice(&result[..32]);
    HexHash::new(hashed_seed)
}

/// The configuration for Mock Da.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    /// Configures failure injection for testing.
    /// Defaults to `FailureBehavior::None` (no failures).
    #[serde(default)]
    pub failure_behavior: FailureBehavior,
}

impl PartialEq for MockDaConfig {
    fn eq(&self, other: &Self) -> bool {
        let basic_eq = self.connection_string == other.connection_string
            && self.sender_address == other.sender_address
            && self.finalization_blocks == other.finalization_blocks
            && self.block_producing == other.block_producing
            && self.randomization == other.randomization
            && self.failure_behavior == other.failure_behavior;

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
            failure_behavior: FailureBehavior::None,
        }
    }

    /// Connection string for in-memory SQLite.
    pub fn sqlite_in_memory() -> String {
        "sqlite::memory:".to_string()
    }

    /// Builds SQLite connection string and checks if a given directory exists.
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
            failure_behavior: FailureBehavior::None,
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
            failure_behavior: FailureBehavior::None,
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
