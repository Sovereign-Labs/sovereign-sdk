use std::num::NonZero;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// See [`SequencerConfig::sequencer_kind_config`].
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SequencerKindConfig {
    /// A "Standard" sequencer, which can post transactions to the rollup but not give soft confirmations.
    Standard(StdSequencerConfig),
    /// A "Preferred" sequencer which is allowed to give soft confirmations.
    Preferred(PreferredSequencerConfig),
}

impl Default for SequencerKindConfig {
    fn default() -> Self {
        SequencerKindConfig::Preferred(Default::default())
    }
}

/// Sequencer configuration.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(
    bound = "Address: JsonSchema, Sc: JsonSchema",
    rename = "SequencerConfig"
)]
pub struct SequencerConfig<Address, Sc = SequencerKindConfig> {
    /// When enabled, submitted transactions are periodically assembled into
    /// batches and automatically posted to the DA layer. When disabled, the
    /// batch production endpoint has to be called explicitly.
    #[serde(default = "default_automatic_batch_production")]
    pub automatic_batch_production: bool,
    /// The sequencer won't process incoming requests unless the node is within
    /// this many blocks or ahead of the sequencer.
    pub max_allowed_node_distance_behind: u64,
    /// For how many seconds the sequencer keeps track of dropped transactions
    /// after being done with them.
    ///
    /// Larger values result in higher memory usage, but better tx status
    /// tracking for users.
    #[serde(default = "default_sequencer_dropped_tx_ttl_secs")]
    pub dropped_tx_ttl_secs: u64,
    /// Rollup address of the sequencer.
    pub rollup_address: Address,
    /// The list of addresses that are allowed to perform admin operations on
    /// the sequencer.
    // The custom "default" is equivalent to Serde's default default, but
    // without the bound `Address: Default`.
    #[serde(default = "Vec::<Address>::new")]
    pub admin_addresses: Vec<Address>,
    /// Sequencer-type specific configuration.
    #[serde(flatten)]
    pub sequencer_kind_config: Sc,
    /// Maximum size of a batch.
    pub max_batch_size_bytes: usize,
    /// Maximum number of blobs sent in parallel.
    pub max_concurrent_blobs: usize,
    /// Maximum time in seconds to wait for a blob to be processed.
    pub blob_processing_timeout_secs: u64,
}

fn default_automatic_batch_production() -> bool {
    true
}

impl<Addr: Clone, BbConfig> SequencerConfig<Addr, BbConfig> {
    /// Replaces the value of [`SequencerConfig::sequencer_kind_config`].
    pub fn with_seq_config<Sc2>(&self, seq_config: Sc2) -> SequencerConfig<Addr, Sc2> {
        SequencerConfig {
            automatic_batch_production: self.automatic_batch_production,
            dropped_tx_ttl_secs: self.dropped_tx_ttl_secs,
            rollup_address: self.rollup_address.clone(),
            max_allowed_node_distance_behind: self.max_allowed_node_distance_behind,
            admin_addresses: self.admin_addresses.clone(),
            max_batch_size_bytes: self.max_batch_size_bytes,
            max_concurrent_blobs: self.max_concurrent_blobs,
            sequencer_kind_config: seq_config,
            blob_processing_timeout_secs: self.blob_processing_timeout_secs,
        }
    }
}

impl<Addr> SequencerConfig<Addr> {
    /// Returns true if the sequencer uses [`SequencerKindConfig::Preferred`].
    pub fn is_preferred_sequencer(&self) -> bool {
        matches!(
            self.sequencer_kind_config,
            SequencerKindConfig::Preferred(_)
        )
    }
}

fn default_sequencer_dropped_tx_ttl_secs() -> u64 {
    60
}

/// Strategy for handling the scenario where the preferred sequencer finds itself close to or past
/// deferred_slots_count in the past, i.e. risking its soft confirmations being invalidated due to
/// the possibility of a non-preferred (deferred) batch having been included.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RecoveryStrategy {
    /// Do not attempt recovery, shutdown the sequencer instead. The user may attempt to resume
    /// operation either by swapping to TryToSave, or deleting everything from the preferred
    /// sequencer database (cancelling ALL pending soft confirmations!).
    None,
    /// Attempt to recover by flushing batches and catching up with the chain. Triggers a bit more
    /// conservatively to attempt to preserve soft confirmations (but if the sequencer was offline,
    /// this will likely make no difference). If some soft confirmations have indeed been
    /// invalidated, the sequencer will be penalized for every invalid batch!
    TryToSave,
}

/// Configuration for [`PreferredSequencer`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub struct PreferredSequencerConfig {
    /// The minimum fee that the preferred sequencer is willing to accept, denominated in rollup tokens. Defaults to zero.
    /// Sequencers should set this to a non-zero value if they wish to cover their DA costs.
    #[serde(default)]
    pub minimum_profit_per_tx: u128,
    /// The size of the Tokio channel used to stream events.
    ///
    /// Don't deviate from the default unless you know what you're doing.
    #[serde(default = "default_events_channel_size")]
    pub events_channel_size: usize,
    /// Optional. When present, Postgres will be used as a database instead of
    /// RocksDB.
    #[serde(default)]
    pub postgres_connection_string: Option<String>,
    /// When enabled, the sequencer will skip some expensive consistency checks
    /// on the state root. This means that bugs in the implementation are less likely to be detected
    /// but may improve performance and allows the sequencer to continue operating in case of known bugs.
    #[serde(default)]
    pub disable_state_root_consistency_checks: bool,
    /// The ideal lag behind the finalized slot number.
    #[serde(default = "default_ideal_lag_behind_finalized_slot")]
    pub ideal_lag_behind_finalized_slot: u64,
    #[serde(default = "default_db_event_channel_size")]
    /// The number of events that can be buffered in the database event channel while `update_state` is running.
    /// This value needs to be increased at higher TPS to avoid blocking the sequencer.
    pub db_event_channel_size: usize,
    /// Strategy for handling recovery scenarios in the preferred sequencer.
    pub recovery_strategy: RecoveryStrategy,
    /// Target time in milliseconds to spend executing all the txs in a single batch. Batches will be closed when they exceed this value.
    pub batch_execution_time_limit_millis: u64,
    /// When enabled, the sequencer runs in replica mode and cannot accept transactions.
    /// It will sync from the master sequencer's database but remain read-only.
    #[serde(default)]
    pub is_replica: bool,
}

impl Default for PreferredSequencerConfig {
    fn default() -> Self {
        Self {
            minimum_profit_per_tx: 0,
            events_channel_size: default_events_channel_size(),
            postgres_connection_string: None,
            disable_state_root_consistency_checks: false,
            ideal_lag_behind_finalized_slot: default_ideal_lag_behind_finalized_slot(),
            recovery_strategy: RecoveryStrategy::None,
            is_replica: false,
            db_event_channel_size: default_db_event_channel_size(),
            batch_execution_time_limit_millis: 6_000, // 6 seconds
        }
    }
}

/// The ideal buffer of finalized slots that the sequencer should maintain. The larger this number,
/// the longer forced transactions will take to be included but the more the sequencer is able to buffer
/// instability on the DA layer.
pub const fn default_ideal_lag_behind_finalized_slot() -> u64 {
    10
}

fn default_events_channel_size() -> usize {
    10_000
}

fn default_db_event_channel_size() -> usize {
    10_000
}

/// Configuration for [`StdSequencer`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct StdSequencerConfig {
    /// Maximum number of transactions in mempool. Once this limit is reached,
    /// the batch builder will evict older transactions.
    pub mempool_max_txs_count: Option<NonZero<usize>>,
    /// Maximum size of a batch. The sequencer will not build batches larger
    /// than this size.
    pub max_batch_size_bytes: Option<NonZero<usize>>,
}
