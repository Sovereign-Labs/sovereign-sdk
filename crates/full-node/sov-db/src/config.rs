use nomt::Options;
pub use rockbound::VersionedColumnFamilyKind;
use rockbound::{rocksdb, CfDescriptorBuilder};
use schemars::JsonSchema;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

use crate::rocks_db_config;
use crate::storage_manager::DEFAULT_MAX_PRUNING_BATCH_SIZE;

type RocksdbOptionsCustomizationFn = dyn Fn(RocksDbKind, &mut rocksdb::Options) + Send + Sync;
type RocksdbCfCustomizationFn = dyn Fn(RocksDbKind, &str, Option<VersionedColumnFamilyKind>, &mut CfDescriptorBuilder)
    + Send
    + Sync;
#[derive(Clone)]
/// Additional customization for a logical RocksDB instance.
pub struct RocksdbOptionsCustomization(Arc<RocksdbOptionsCustomizationFn>);

impl RocksdbOptionsCustomization {
    /// Create a new RocksDB options customizer.
    pub fn new(
        customize: impl Fn(RocksDbKind, &mut rocksdb::Options) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(customize))
    }

    pub(crate) fn apply(&self, db_kind: RocksDbKind, options: &mut rocksdb::Options) {
        (self.0)(db_kind, options);
    }
}

impl std::fmt::Debug for RocksdbOptionsCustomization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RocksdbOptionsCustomization")
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
/// Logical RocksDB instances managed by `sov-db`.
pub enum RocksDbKind {
    /// Ledger history database.
    Ledger,
    /// Accessory state database.
    Accessory,
    /// Flat state live database.
    FlatStateLive,
    /// Flat state archival database.
    FlatStateArchival,
}

#[derive(Clone)]
/// Customization hook for column-family and table options.
pub struct RocksdbCfCustomization(Arc<RocksdbCfCustomizationFn>);

impl RocksdbCfCustomization {
    /// Create a new column-family customization hook.
    pub fn new(
        customize: impl Fn(RocksDbKind, &str, Option<VersionedColumnFamilyKind>, &mut CfDescriptorBuilder)
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self(Arc::new(customize))
    }

    pub(crate) fn apply(
        &self,
        db_kind: RocksDbKind,
        cf_name: &str,
        versioned_kind: Option<VersionedColumnFamilyKind>,
        builder: &mut CfDescriptorBuilder,
    ) {
        (self.0)(db_kind, cf_name, versioned_kind, builder);
    }
}

impl std::fmt::Debug for RocksdbCfCustomization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RocksdbCfCustomization")
    }
}

/// State-version pruning policy: how (and whether) old historical versions of state keys
/// are deleted.
///
/// Defaults to [`PrunerConfig::Off`] (full history retained). In config files, omitting the
/// `pruner` section disables pruning; otherwise select a variant, e.g.
/// `[storage.pruner.periodic]` or `[storage.pruner.once_at_startup]`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PrunerConfig {
    /// Never prune; the node retains full history.
    #[default]
    Off,
    /// Prune exactly once, synchronously, at node startup (before block processing begins),
    /// then never again during this run. This performs all pruning I/O up front, when there
    /// is no live read/write traffic to compete with, instead of interfering on every
    /// finalized block as [`PrunerConfig::Periodic`] does.
    OnceAtStartup {
        /// Number of recent versions to retain for historical querying.
        versions_to_keep: NonZeroU64,
        /// Maximum number of keys deleted per internal batch. Omitting it in config falls back
        /// to [`default_max_pruning_batch_size`].
        ///
        /// Within each pass the user-state pruner is served first and the kernel-state pruner
        /// only gets the leftover budget, so while user state has more than `max_batch_size`
        /// prunable keys the kernel pruner waits for user pruning to catch up. This only adds
        /// passes here, since the startup prune runs to completion before block processing; the
        /// disk-growth effect of this ordering is a steady-state [`PrunerConfig::Periodic`]
        /// concern.
        #[serde(default = "default_max_pruning_batch_size")]
        max_batch_size: NonZeroUsize,
        /// If `true`, run a full RocksDB compaction on the pruned column families after the
        /// startup prune completes, dropping the resulting tombstones and reclaiming disk
        /// space. This is heavy (it rewrites the affected column families) and only sensible
        /// for the one-time startup prune, so it is off by default.
        #[serde(default)]
        compact_after: bool,
    },
    /// Periodically prune in the background during finalization (the historical behavior):
    /// roughly every `block_interval` finalized DA blocks a background pruner is spawned and
    /// its delete batch is committed on a subsequent finalize.
    Periodic {
        /// Run the pruner roughly every `block_interval` finalized DA blocks.
        block_interval: u64,
        /// Number of recent versions to retain for historical querying.
        versions_to_keep: NonZeroU64,
        /// Maximum number of keys deleted per batch. Omitting it in config falls back to
        /// [`default_max_pruning_batch_size`].
        ///
        /// Within each pass the user-state pruner is served first and the kernel-state pruner
        /// only gets the leftover budget, so while user state has more than `max_batch_size`
        /// prunable keys the kernel pruner makes no progress and kernel historical state can
        /// grow on disk until user pruning catches up; raising `max_batch_size` shortens that
        /// window.
        #[serde(default = "default_max_pruning_batch_size")]
        max_batch_size: NonZeroUsize,
    },
}

/// Default per-batch key cap used when `max_batch_size` is omitted from config.
///
/// Serde calls this to fill in the `max_batch_size` field of [`PrunerConfig::Periodic`] /
/// [`PrunerConfig::OnceAtStartup`] when the key is absent.
pub fn default_max_pruning_batch_size() -> NonZeroUsize {
    NonZeroUsize::new(DEFAULT_MAX_PRUNING_BATCH_SIZE)
        .expect("DEFAULT_MAX_PRUNING_BATCH_SIZE is non-zero")
}

/// Configuration for Sovereign Rollup node database.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RollupDbConfig {
    /// Path where all databases are stored
    pub path: std::path::PathBuf,
    /// Optional custom path for the ledger database.
    ///
    /// When set, ledger DB data is stored at this path. When not set, the ledger DB is stored
    /// under [`RollupDbConfig::path`] (the existing behavior).
    pub ledger_db_path: Option<std::path::PathBuf>,
    /// The size of the cache which holds hot key-value pairs in the flat state database. Default is 1 GiB.
    /// A larger cache size can improve execution speed at the cost of more memory usage.
    pub state_cache_size: Option<usize>,

    /// Number of concurrent commit workers for the user state.
    /// More details at [`Options::commit_concurrency`]
    pub user_commit_concurrency: Option<usize>,
    /// Value is determined by the expected size of the state. Recommended to start with 15_000_000.
    /// Can be increased for an existing database, but cannot be decreased.
    /// More details at [`Options::hashtable_buckets`]
    pub user_hashtable_buckets: Option<u32>,
    /// Sets whether to preallocate the hashtable file for user db.
    /// More details at [`Options::preallocate_ht`]
    pub user_preallocate_ht: Option<bool>,
    /// Page cache size for user state.
    /// More details at [`Options::page_cache_size`]
    pub user_page_cache_size: Option<usize>,
    /// Leaf cache size for user state.
    /// More details at [`Options::leaf_cache_size`]
    pub user_leaf_cache_size: Option<usize>,
    /// Page cache upper levels for user state.
    /// More details at [`Options::page_cache_upper_levels`]
    pub user_page_cache_upper_levels: Option<usize>,

    // Kernel state configuration
    /// Number of concurrent commit workers for the kernel state.
    /// More details at [`Options::commit_concurrency`]
    pub kernel_commit_concurrency: Option<usize>,
    /// Can be increased for an existing database, but cannot be decreased.
    /// More details at [`Options::hashtable_buckets`]
    pub kernel_hashtable_buckets: Option<u32>,
    /// Sets whether to preallocate the hashtable file for kernel db.
    /// More details at [`Options::preallocate_ht`]
    pub kernel_preallocate_ht: Option<bool>,
    /// Page cache size for kernel state.
    /// More details at [`Options::page_cache_size`]
    pub kernel_page_cache_size: Option<usize>,
    /// Leaf cache size for kernel state.
    /// More details at [`Options::leaf_cache_size`]
    pub kernel_leaf_cache_size: Option<usize>,
    /// Page cache upper levels for kernel state.
    /// More details at [`Options::page_cache_upper_levels`]
    pub kernel_page_cache_upper_levels: Option<usize>,

    /// State-version pruning policy. Defaults to [`PrunerConfig::Off`] (full history
    /// retained) when omitted from config.
    ///
    /// The struct uses `#[serde(deny_unknown_fields)]`, so the deprecated flat
    /// `pruner_block_interval` / `pruner_versions_to_keep` / `pruner_max_batch_size` keys (and any
    /// other unknown key) are rejected at parse time rather than silently ignored.
    #[serde(default)]
    pub pruner: PrunerConfig,
}

impl RollupDbConfig {
    /// Helper for development
    #[cfg(feature = "test-utils")]
    pub fn default_in_path(path: std::path::PathBuf) -> Self {
        Self {
            path,
            ledger_db_path: None,
            state_cache_size: Some(1_000_000), // Use a 1MB state cache for tests
            user_commit_concurrency: Some(2),
            user_hashtable_buckets: Some(if cfg!(debug_assertions) {
                500
            } else {
                1_000_000
            }),
            user_preallocate_ht: Some(false),
            user_page_cache_size: Some(16),
            user_leaf_cache_size: Some(16),
            user_page_cache_upper_levels: None,
            kernel_commit_concurrency: Some(2),
            kernel_hashtable_buckets: None,
            kernel_preallocate_ht: Some(false),
            kernel_page_cache_size: Some(16),
            kernel_leaf_cache_size: Some(16),
            kernel_page_cache_upper_levels: None,
            pruner: PrunerConfig::Off,
        }
    }

    /// Kernel state is smaller, so we can have less required options.
    /// But it has rollback enabled for 2 database commit support
    pub(crate) fn get_kernel_options(&self) -> Options {
        let mut opts = nomt_default_options();
        // Enable rollback, so we can handle errors with commits to 2 databases.
        opts.rollback(true);
        opts.max_rollback_log_len(1);
        opts.commit_concurrency(
            self.kernel_commit_concurrency
                .expect("`kernel_commit_concurrency` concurrency must be set"),
        );
        opts.hashtable_buckets(self.kernel_hashtable_buckets());

        if let Some(preallocate_ht) = self.kernel_preallocate_ht {
            opts.preallocate_ht(preallocate_ht);
        }

        if let Some(page_cache_size) = self.kernel_page_cache_size {
            opts.page_cache_size(page_cache_size);
        }
        if let Some(leaf_cache_size) = self.kernel_leaf_cache_size {
            opts.leaf_cache_size(leaf_cache_size);
        }
        if let Some(page_cache_upper_levels) = self.kernel_page_cache_upper_levels {
            opts.page_cache_upper_levels(page_cache_upper_levels);
        }

        opts.path(self.path.join("kernel_nomt_db"));

        opts
    }

    pub(crate) fn kernel_hashtable_buckets(&self) -> u32 {
        self.kernel_hashtable_buckets.unwrap_or({
            if cfg!(debug_assertions) {
                // 2MB
                500
            } else {
                // 1000MB
                256_000
            }
        })
    }

    pub(crate) fn get_user_options(&self) -> Options {
        let mut opts = nomt_default_options();
        // Enabling rollback for user space too, to be able to sync with the historical state.
        opts.rollback(true);
        opts.max_rollback_log_len(1);
        opts.commit_concurrency(
            self.user_commit_concurrency
                .expect("`user_commit_concurrency` must be set"),
        );
        opts.hashtable_buckets(self.user_hashtable_buckets());
        if let Some(preallocate_ht) = self.user_preallocate_ht {
            opts.preallocate_ht(preallocate_ht);
        }
        if let Some(page_cache_size) = self.user_page_cache_size {
            opts.page_cache_size(page_cache_size);
        }
        if let Some(leaf_cache_size) = self.user_leaf_cache_size {
            opts.leaf_cache_size(leaf_cache_size);
        }
        if let Some(page_cache_upper_levels) = self.user_page_cache_upper_levels {
            opts.page_cache_upper_levels(page_cache_upper_levels);
        }

        opts.path(self.path.join("user_nomt_db"));
        opts
    }

    pub(crate) fn user_hashtable_buckets(&self) -> u32 {
        self.user_hashtable_buckets
            .expect("`user_hashtable_buckets` must be set")
    }

    pub(crate) fn pruner(&self) -> PrunerConfig {
        self.pruner
    }
}

/// Open-time wrapper for [`RollupDbConfig`] that carries runtime-only RocksDB customizations.
#[derive(Debug, Clone)]
pub struct RollupDbConfigWithCustomizations {
    config: RollupDbConfig,
    rocksdb_options: Option<RocksdbOptionsCustomization>,
    rocksdb_cf_options: Option<RocksdbCfCustomization>,
}

impl RollupDbConfigWithCustomizations {
    /// Create a new open-time wrapper around a pure [`RollupDbConfig`].
    pub fn new(config: RollupDbConfig) -> Self {
        Self {
            config,
            rocksdb_options: None,
            rocksdb_cf_options: None,
        }
    }

    /// Set DB-wide RocksDB customization shared across all logical `sov-db` instances.
    pub fn with_rocksdb_options(mut self, customization: RocksdbOptionsCustomization) -> Self {
        self.rocksdb_options = Some(customization);
        self
    }

    /// Set column-family and table customization shared across all logical `sov-db` instances.
    pub fn with_rocksdb_cf_options(mut self, customization: RocksdbCfCustomization) -> Self {
        self.rocksdb_cf_options = Some(customization);
        self
    }

    /// Borrow the underlying pure data config.
    pub fn config(&self) -> &RollupDbConfig {
        &self.config
    }

    /// Discard runtime-only customizations and return the underlying data config.
    pub fn into_config(self) -> RollupDbConfig {
        self.config
    }

    pub(crate) fn get_rocksdb_options(&self, db_kind: RocksDbKind) -> rocksdb::Options {
        rocks_db_config::gen_rocksdb_options_with(&Default::default(), false, |options| {
            if let Some(customization) = &self.rocksdb_options {
                customization.apply(db_kind, options);
            }
        })
    }

    pub(crate) fn customize_rocksdb_cf(
        &self,
        db_kind: RocksDbKind,
        cf_name: &str,
        versioned_kind: Option<VersionedColumnFamilyKind>,
        builder: &mut CfDescriptorBuilder,
    ) {
        if let Some(customization) = &self.rocksdb_cf_options {
            customization.apply(db_kind, cf_name, versioned_kind, builder);
        }
    }
}

impl From<RollupDbConfig> for RollupDbConfigWithCustomizations {
    fn from(config: RollupDbConfig) -> Self {
        Self::new(config)
    }
}

fn nomt_default_options() -> Options {
    let mut opts = Options::new();
    opts.metrics(true);
    opts
}
