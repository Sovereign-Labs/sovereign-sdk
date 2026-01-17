use std::{net::IpAddr, num::NonZero};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// See [`SequencerConfig::sequencer_kind_config`].
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum SequencerKindConfig<Address: Copy> {
    /// A "Standard" sequencer, which can post transactions to the rollup but not give soft confirmations.
    Standard(StdSequencerConfig),
    /// A "Preferred" sequencer which is allowed to give soft confirmations.
    Preferred(PreferredSequencerConfig<Address>),
}

impl<Address: Copy + serde::Serialize + serde::de::DeserializeOwned> Default
    for SequencerKindConfig<Address>
{
    fn default() -> Self {
        SequencerKindConfig::Preferred(PreferredSequencerConfig::default())
    }
}

/// Configuration data used by sequencer extensions, such as EVM endpoints.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Copy)]
pub struct SeqConfigExtension {
    pub max_log_limit: usize,
    /// The maximum size of the response to the eth_getLogs RPC endpoint. Use 1MB - 30KB for a safe default.
    #[serde(default = "default_response_size_limit")]
    pub response_size_limit: usize,
}

fn default_response_size_limit() -> usize {
    (1024 * 1024) - (1024 * 30) // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
}

/// Sequencer configuration.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(rename = "SequencerConfig")]
#[serde(deny_unknown_fields)]
pub struct SequencerConfig<Address: Copy, Sc = SequencerKindConfig<Address>> {
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
    /// Maximum time in seconds to wait for a blob to be processed, since it has been published to DA.
    pub blob_processing_timeout_secs: u64,
    /// Extensions to the sequencer config (for example evm related configuration).
    pub extension: Option<SeqConfigExtension>,
}

fn default_automatic_batch_production() -> bool {
    true
}

impl<Addr: Copy + Clone, BbConfig> SequencerConfig<Addr, BbConfig> {
    /// Replaces the value of [`SequencerConfig::sequencer_kind_config`].
    pub fn with_seq_config<Sc2>(&self, seq_config: Sc2) -> SequencerConfig<Addr, Sc2> {
        SequencerConfig {
            automatic_batch_production: self.automatic_batch_production,
            dropped_tx_ttl_secs: self.dropped_tx_ttl_secs,
            rollup_address: self.rollup_address,
            max_allowed_node_distance_behind: self.max_allowed_node_distance_behind,
            admin_addresses: self.admin_addresses.clone(),
            max_batch_size_bytes: self.max_batch_size_bytes,
            max_concurrent_blobs: self.max_concurrent_blobs,
            sequencer_kind_config: seq_config,
            blob_processing_timeout_secs: self.blob_processing_timeout_secs,
            extension: self.extension,
        }
    }
}

impl<Addr: Copy> SequencerConfig<Addr> {
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

#[derive(Debug, Copy, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub enum NodeRole {
    /// This node runs as the leader. The leader is responsible for producing new batches.
    Leader,
    /// This node runs as a replica, syncing its state from the leader.
    /// Replicas do not publish blobs to the DA layer, do not accept transactions via the API,
    /// and do not issue soft confirmations.
    Replica,
    /// This node runs in replica mode with Postgres-based synchronization from the leader disabled.
    /// It receives transactions exclusively through the DA layer.
    /// Use this mode when you want replica behavior without accepting transactions from the Leader only from the DA.
    ReplicaNoLeaderSync,
    /// The node initially starts as a `Replica` and attempts to register itself as the `Leader`
    /// by sending a request to the Db to acquire leadership.
    /// If successful, it becomes the `Leader` and all other nodes remain Replicas.
    /// If the Leader goes down and fails to refresh its entry in the `Leader` table,
    /// one of the `Replicas` will take over and assume the Leader role.
    DbElected,
}

/// Postgres DB config.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub struct PostgresConfig {
    /// Connection string.
    pub postgres_connection_string: String,
    /// Id of the node.
    pub node_id: String,
    #[allow(missing_docs)]
    pub node_role: NodeRole,
}

/// Configuration for [`PreferredSequencer`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreferredSequencerConfig<Address: Copy> {
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
    pub postgres_config: Option<PostgresConfig>,
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
    #[serde(default = "default_num_cache_warmup_workers")]
    /// The number of workers that warm up the main executor cache.
    pub num_cache_warmup_workers: usize,
    /// Configuration for the timing oracle.
    #[serde(default)]
    pub timing_oracle: Option<TimingOracleConfig>,
    /// Configuration for rate-limiting the sequencer.
    #[serde(default = "default_rate_limiter::<Address>")]
    pub rate_limiter: Option<SovRateLimiterConfig<Address>>,
    /// The fartherst nonce into the future that the sequencer will accept and queue. This directly
    /// impacts the maximum "batch" of transactions that can be simultaneously sent to the
    /// sequencer out of order.
    #[serde(default = "default_maximum_future_nonce_delta")]
    pub maximum_future_nonce_delta: u64,
    /// The timeout after which a transaction with a near-future nonce (bounded by
    /// `maximum_future_nonce_delta`) will be dropped and forgotten by the sequencer. Increasing
    /// this value increases the leniency with which out-of-order transactions are considered a
    /// "simultaneous batch" which should be reordered, but can increase memory usage and queue
    /// lock contention.
    #[serde(default = "default_future_nonce_transaction_timeout_millis")]
    pub future_nonce_transaction_timeout_millis: u64,
}

impl<Address: Copy> Default for PreferredSequencerConfig<Address> {
    fn default() -> Self {
        Self {
            minimum_profit_per_tx: 0,
            events_channel_size: default_events_channel_size(),
            postgres_config: None,
            disable_state_root_consistency_checks: false,
            ideal_lag_behind_finalized_slot: default_ideal_lag_behind_finalized_slot(),
            recovery_strategy: RecoveryStrategy::None,
            db_event_channel_size: default_db_event_channel_size(),
            batch_execution_time_limit_millis: 6_000, // 6 seconds
            num_cache_warmup_workers: default_num_cache_warmup_workers(),
            maximum_future_nonce_delta: default_maximum_future_nonce_delta(),
            future_nonce_transaction_timeout_millis:
                default_future_nonce_transaction_timeout_millis(),
            timing_oracle: None,
            rate_limiter: None,
        }
    }
}

const fn default_rate_limiter<Address: Copy>() -> Option<SovRateLimiterConfig<Address>> {
    None
}

pub const fn default_maximum_future_nonce_delta() -> u64 {
    100
}

pub const fn default_future_nonce_transaction_timeout_millis() -> u64 {
    2000
}

pub const fn default_num_cache_warmup_workers() -> usize {
    3
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
#[serde(deny_unknown_fields)]
pub struct StdSequencerConfig {
    /// Maximum number of transactions in mempool. Once this limit is reached,
    /// the batch builder will evict older transactions.
    pub mempool_max_txs_count: Option<NonZero<usize>>,
    /// Maximum size of a batch. The sequencer will not build batches larger
    /// than this size.
    pub max_batch_size_bytes: Option<NonZero<usize>>,
}

// Configuration for the timing oracle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub struct TimingOracleConfig {
    /// The priority fee percentage that the sequencer will pay for the timestamp oracle update tx.
    pub priority_fee_percentage: u8,
    /// The maximum fee that the sequencer will pay for the timestamp oracle update tx.
    pub max_fee: u64,
    /// The interval in milliseconds at which the timestamp oracle update tx is submitted.
    pub interval_millis: u64,
    /// The private key to use to sign timestamp oracle txs. If none is provided, an ephemeral key will be generated.
    pub private_key_hex: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub struct SovRateLimiterConfig<Address: Copy> {
    /// The cache size for the rate limiter.
    pub max_nb_of_concurrent_users_in_rate_limiter: u64,
    /// The maximum number of requests allowed per second.
    pub max_requests_per_second: u64,
    /// Default limits.
    pub default_limits: Limits,
    pub address_custom_limits: Vec<(Address, Limits)>,
    pub ip_custom_limits: Vec<(IpAddr, Limits)>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
pub struct Limits {
    /// Determines the threshold for rate-limiting requests.
    /// The resources allocated per user per rate-limiting bucket, defined in units of 1/1000th of a full batch. E.g. resources_per_bucket = 10 would mean each bucket allows the user to use 1% of a full batch capacity.
    pub resources_per_bucket: u64,
    /// The refill rate of buckets. E.g. if refill_rate = 5, the user's rate limiting bucket will be refilled up to five times every batch.
    /// Values between 1 and 20 are recommended starting points.
    pub refill_rate: u64,
}
