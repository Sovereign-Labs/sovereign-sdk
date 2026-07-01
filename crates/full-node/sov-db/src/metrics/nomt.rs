use std::io::Write;

use nomt::hasher::BinaryHasher;
use nomt::Nomt;
use sov_metrics::Metric;
use sov_rollup_interface::reexports::digest;

/// Snapshot of NOMT hash-table occupancy and page-cache effectiveness for one database.
/// The `db` tag disambiguates instances (user / kernel / ledger). Emitted as `sov_nomt_db_stats`.
///
/// **What healthy looks like:** `hash_table_occupied / hash_table_capacity < 0.9`, page cache miss
/// ratio low and steady, page/value fetch times flat.
///
/// **Diagnostic signals:**
/// - Occupancy > 0.9 → NOMT emits a warning log; hash collisions start to degrade lookups
///   and inserts. Remediation: resync the database with a larger `hash_table_capacity`.
/// - `page_cache_misses / page_requests` rising → working set has outgrown the page cache.
///   Remediation: raise the NOMT page-cache size or add RAM.
/// - `avg_page_fetch_time_ns` spiking while miss ratio is flat → underlying disk is saturated
///   (compare with OS-level I/O metrics and other DBs' NOMT stats).
///
/// **Correlate with:** `sov_nomt_commit_detailed` (slow commits often trace back here) and
/// `sov_storage_manager_finalization` (finalization commit_time).
#[derive(Debug)]
pub struct NomtDbMetric {
    /// Logical name of the NOMT instance (user/kernel/ledger); used as the InfluxDB `db` tag.
    pub db: &'static str,
    pub hash_table_capacity: usize,
    pub hash_table_occupied: usize,
    pub page_requests: u64,
    pub page_cache_misses: u64,
    pub avg_page_fetch_time_ns: Option<u64>,
    pub avg_value_fetch_time_ns: Option<u64>,
}

impl NomtDbMetric {
    pub fn new<H>(db: &'static str, nomt: &Nomt<BinaryHasher<H>>) -> Self
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    {
        let hash_table_utilization = nomt.hash_table_utilization();
        if hash_table_utilization.occupancy_rate() > 0.9 {
            tracing::warn!(
                %db,
                rate = hash_table_utilization.occupancy_rate(),
                "Occupancy rate for NOMT hashtable is too high. Please update buckets size and resync database");
        }
        let metrics = nomt.metrics();
        Self {
            db,
            hash_table_capacity: hash_table_utilization.capacity,
            hash_table_occupied: hash_table_utilization.occupied,
            page_requests: metrics.get_page_requests(),
            page_cache_misses: metrics.get_page_cache_misses(),
            avg_page_fetch_time_ns: metrics.get_page_fetch_time(),
            avg_value_fetch_time_ns: metrics.get_value_fetch_time(),
        }
    }
}

impl Metric for NomtDbMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_nomt_db_stats"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // DB as tag, rest as fields
        write!(
            buffer,
            "{},db={} ht_capacity={},ht_occupied={},page_requests={},page_cache_misses={}",
            self.measurement_name(),
            self.db,
            self.hash_table_capacity,
            self.hash_table_occupied,
            self.page_requests,
            self.page_cache_misses,
        )?;
        if let Some(avg_page_fetch_time) = &self.avg_page_fetch_time_ns {
            write!(buffer, ",avg_page_fetch_time_ns={avg_page_fetch_time}")?;
        }
        if let Some(avg_value_fetch_time) = &self.avg_value_fetch_time_ns {
            write!(buffer, ",avg_value_fetch_time_ns={avg_value_fetch_time}")?;
        }

        Ok(())
    }
}

/// Per-slot cost of opening a NOMT session and the unfinalized-overlay depth at that moment.
/// Emitted as `sov_nomt_begin_session`.
///
/// **What healthy looks like:** `init_time` low and flat; `overlays` bounded by the finalization
/// window (i.e., stays near the configured `STATE_ROOT_DELAY_BLOCKS`).
///
/// **Diagnostic signals:**
/// - `overlays` monotonically increasing → finalization is falling behind (the node cannot
///   promote unfinalized state to disk fast enough). Check `sov_rollup_runner_da.sync_distance`
///   and `sov_runner_process_stf_changes`.
/// - `init_time` spiking with stable `overlays` → storage-engine contention at session start
///   (often correlated with compaction or heavy commits on the same DB).
///
/// **Correlate with:** `sov_storage_manager_finalization` (finalization latency drives overlays),
/// `sov_nomt_commit_detailed` (commits can block session starts on the same DB).
#[derive(Debug)]
pub struct NomtBeginSessionMetric {
    /// Logical name of the NOMT instance (user/kernel/ledger); InfluxDB `db` tag.
    pub db: &'static str,
    /// Number of unfinalized overlays stacked on top of the on-disk state when the session opened.
    /// This is the "lag to finalization" in slots.
    pub overlays: usize,
    pub init_time: std::time::Duration,
}

impl Metric for NomtBeginSessionMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_nomt_begin_session"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // DB as tag, rest as fields
        write!(
            buffer,
            "{},db={} overlays={},init_time_us={}",
            self.measurement_name(),
            self.db,
            self.overlays,
            self.init_time.as_micros(),
        )
    }
}

/// Wall-clock breakdown of finalizing one slot in the storage manager (promoting the oldest
/// overlay to on-disk state). Emitted as `sov_storage_manager_finalization`.
///
/// **What healthy looks like:** all three fields sub-second and stable from slot to slot.
///
/// **Diagnostic signals:**
/// - `commit_time` dominates slot time → storage is the rollup's bottleneck. Investigate
///   `sov_nomt_db_stats` (disk/page-cache pressure) and `sov_nomt_commit_detailed`.
/// - `pruning_commit_time` is `Some` and consistently large → pruner backlog; cross-check
///   `sov_db_pruner` throughput and the rollup's retention configuration.
/// - `preparation_time` rising → large overlays are being materialized; see
///   `sov_nomt_begin_session.overlays` and `sov_state_db_materialization`.
#[derive(Debug)]
pub struct StorageManagerFinalizationMetric {
    pub preparation_time: std::time::Duration,
    pub commit_time: std::time::Duration,
    /// `None` when pruning did not run this slot; `Some(_)` otherwise.
    pub pruning_commit_time: Option<std::time::Duration>,
}

impl Metric for StorageManagerFinalizationMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_storage_manager_finalization"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} prep_time_us={},commit_time_us={}",
            self.measurement_name(),
            self.preparation_time.as_micros(),
            self.commit_time.as_micros(),
        )?;

        if let Some(pruning_time) = &self.pruning_commit_time {
            write!(
                buffer,
                ",pruning_commit_time_us={}",
                pruning_time.as_micros()
            )?;
        }

        Ok(())
    }
}

/// Throughput and efficiency of one pruner pass over a NOMT database. Emitted as `sov_db_pruner`,
/// one point per database per pass. The `db` tag distinguishes which database was pruned: `user`
/// and `kernel` (the historical / versioned state, pruned via rockbound's `VersionedDB`) and
/// `ModuleAccessoryState` (the accessory state).
///
/// **What healthy looks like:** `time` roughly linear in `keys_inspected`; a non-trivial
/// `keys_to_prune / keys_inspected` ratio (the pruner finds work on every pass).
///
/// **Diagnostic signals:**
/// - Sustained `keys_to_prune ≈ 0` with non-zero `keys_inspected` → pruner is scanning but
///   finding nothing: retention config may be wrong, or nothing is eligible for pruning yet.
/// - `keys_inspected` flat while `time` spikes → disk bottleneck on this DB (check
///   `sov_nomt_db_stats` page fetch times).
/// - No emissions at all over long windows → the pruner task may be stuck; confirm the
///   pruner background task is still alive.
///
/// **Correlate with:** `sov_storage_manager_finalization.pruning_commit_time` (the commit
/// cost paired with each inspection pass).
#[derive(Debug)]
pub struct PrunerMetric {
    /// Logical name of the database being pruned (`user`, `kernel`, or `ModuleAccessoryState`);
    /// InfluxDB `db` tag.
    pub db: &'static str,
    /// Base keys / pruning-index entries examined this pass. For `user`/`kernel` this is the count
    /// of pruning-CF entries collected (bounded by the batch-size cap); for `ModuleAccessoryState`
    /// it is the number of unique base keys scanned.
    pub keys_inspected: usize,
    /// Number of key-versions queued for deletion this pass.
    pub keys_to_prune: usize,
    /// Wall-clock time of the collect/scan phase only — not the commit, which is reported as
    /// `sov_storage_manager_finalization.pruning_commit_time`.
    pub time: std::time::Duration,
}

impl Metric for PrunerMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_db_pruner"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},db={} keys_inspected={},keys_to_prune={},time_us={}",
            self.measurement_name(),
            self.db,
            self.keys_inspected,
            self.keys_to_prune,
            self.time.as_micros(),
        )
    }
}

/// Wall-clock cost of the post-prune RocksDB compaction that rewrites the pruned column
/// families to drop their delete tombstones and reclaim disk space. Emitted as
/// `sov_db_compaction`, split per database (`accessory`, `user`, `kernel`).
///
/// **When it fires:** only the one-time startup prune (`PrunerConfig::OnceAtStartup` with
/// `compact_after`) triggers this compaction, so the metric is sparse — expect at most one
/// point per node start. There is no periodic compaction; the tombstones written by periodic
/// pruning are reclaimed by RocksDB's own background compaction, which is not observable here.
///
/// **Diagnostic signals:**
/// - Large values block startup (the node cannot serve traffic until compaction finishes); the
///   total is `accessory + user + kernel`. Cross-check the preceding `sov_db_pruner` pass to see
///   how much was deleted before this compaction ran.
#[derive(Debug)]
pub struct CompactionMetric {
    /// Time spent compacting the accessory-state column family.
    pub accessory_time: std::time::Duration,
    /// Time spent compacting the user historical + pruning column families.
    pub user_time: std::time::Duration,
    /// Time spent compacting the kernel historical + pruning column families.
    pub kernel_time: std::time::Duration,
}

impl Metric for CompactionMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_db_compaction"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} accessory_us={},user_us={},kernel_us={},total_us={}",
            self.measurement_name(),
            self.accessory_time.as_micros(),
            self.user_time.as_micros(),
            self.kernel_time.as_micros(),
            (self.accessory_time + self.user_time + self.kernel_time).as_micros(),
        )
    }
}

#[derive(Debug)]
pub struct MerklizedCommitMetric {
    // How much time in total it took to write overlay
    pub write_user: std::time::Duration,
    pub write_kernel: std::time::Duration,
    // How many attempts it took to write
    pub write_attempts_user: usize,
    pub write_attempts_kernel: usize,
    pub total: std::time::Duration,
}

impl MerklizedCommitMetric {
    fn serialize_values_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // Times
        let write_user_us = self.write_user.as_micros();
        let write_kernel_us = self.write_kernel.as_micros();
        let total_us = self.total.as_micros();
        write!(buffer,
               "merklized_write_user_us={write_user_us},merklized_write_kernel_us={write_kernel_us},merklized_total_us={total_us}")?;

        // Attempts
        let write_attempts_user = self.write_attempts_user;
        let write_attempts_kernel = self.write_attempts_kernel;
        write!(buffer,
               ",merklized_attempts_write_user={write_attempts_user},merklized_write_attempts_kernel={write_attempts_kernel}")
    }
}

#[derive(Debug)]
pub struct FlatStateCommitMetric {
    pub prepare: std::time::Duration,
    pub write: std::time::Duration,
}

#[derive(Debug)]
pub struct CommitDetailedMetric {
    pub merklized_commit: MerklizedCommitMetric,
    pub merklized_commit_from_caller: std::time::Duration,
    pub flat: FlatStateCommitMetric,
    pub accessory_commit: std::time::Duration,
    pub ledger_commit: std::time::Duration,
}

impl Metric for CommitDetailedMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_nomt_commit_detailed"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{} ", self.measurement_name())?;
        self.merklized_commit
            .serialize_values_for_telegraf(buffer)?;
        let merklized_commit_from_caller_us = self.merklized_commit_from_caller.as_micros();
        let flat_prepare_us = self.flat.prepare.as_micros();
        let flat_write_us = self.flat.write.as_micros();
        let accessory_commit_us = self.accessory_commit.as_micros();
        let ledger_commit_us = self.ledger_commit.as_micros();
        write!(buffer, ",merklized_from_caller_us={merklized_commit_from_caller_us},flat_prepare_us={flat_prepare_us},flat_write_us={flat_write_us},accessory_commit_us={accessory_commit_us},ledger_commit_us={ledger_commit_us}")
    }
}
