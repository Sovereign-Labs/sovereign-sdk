//! Pruning policy and runtime for the NOMT storage manager.
//!
//! [`PruningController`] owns everything pruning-related: the schedule resolved from
//! [`PrunerConfig`], the in-flight background pruner job, and the bookkeeping needed to decide
//! when to spawn the next one. The storage manager holds a single [`PruningController`] and
//! delegates to it, rather than spreading pruning state across its own fields.

use std::time::Duration;

use sov_rollup_interface::reexports::digest;

use super::groups::{DbGroup, PrunerJob};
use crate::config::{PrunerConfig, RollupDbConfig};

/// Pruning schedule resolved from [`PrunerConfig`] at construction time: `NonZero` values are
/// unwrapped to plain `usize` and `max_batch_size` is resolved to its concrete default. Each
/// variant carries exactly the parameters that mode needs, so there are no unused/dummy fields.
#[derive(Debug, Clone, Copy)]
enum PrunerSchedule {
    /// Never prune.
    Off,
    /// Prune in the background during finalization, roughly every `block_interval` finalized
    /// blocks.
    Periodic {
        block_interval: u64,
        versions_to_keep: usize,
        max_batch_size: usize,
    },
    /// Prune once, synchronously, at startup, then never again during this run.
    OnceAtStartup {
        versions_to_keep: usize,
        max_batch_size: usize,
        compact_after: bool,
    },
}

impl PrunerSchedule {
    fn from_config(config: PrunerConfig) -> Self {
        match config {
            PrunerConfig::Off => Self::Off,
            PrunerConfig::Periodic {
                block_interval,
                versions_to_keep,
                max_batch_size,
            } => Self::Periodic {
                block_interval,
                versions_to_keep: versions_to_keep.get() as usize,
                max_batch_size: RollupDbConfig::resolve_max_batch_size(max_batch_size),
            },
            PrunerConfig::OnceAtStartup {
                versions_to_keep,
                max_batch_size,
                compact_after,
            } => Self::OnceAtStartup {
                versions_to_keep: versions_to_keep.get() as usize,
                max_batch_size: RollupDbConfig::resolve_max_batch_size(max_batch_size),
                compact_after,
            },
        }
    }
}

/// Owns the pruning schedule and the runtime state needed to drive it (the in-flight background
/// pruner job and the height at which it last finished). Created once per storage manager.
pub(crate) struct PruningController {
    schedule: PrunerSchedule,
    /// The background pruner job spawned by the periodic path, if one is currently running. Its
    /// delete batch is committed on a subsequent [`Self::on_finalize`].
    in_flight: Option<PrunerJob>,
    last_finish_at_height: Option<u64>,

    /// Number of pruning batches committed so far. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    commits_count: u64,
    /// Whether the most recently committed pruning batch hit the size limit. Test-only
    /// observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    last_hit_size_limit: bool,
}

impl PruningController {
    /// Resolves the pruning schedule from `config`. Does not perform any pruning.
    pub(crate) fn new(config: PrunerConfig) -> Self {
        Self {
            schedule: PrunerSchedule::from_config(config),
            in_flight: None,
            last_finish_at_height: None,
            #[cfg(any(test, feature = "test-utils"))]
            commits_count: 0,
            #[cfg(any(test, feature = "test-utils"))]
            last_hit_size_limit: false,
        }
    }

    /// Runs the one-time startup prune synchronously to completion, if the schedule is
    /// [`PrunerConfig::OnceAtStartup`]; a no-op otherwise. Called from the storage manager
    /// constructor, before any state access, while there is no live read/write traffic.
    pub(crate) fn run_startup_prune<H, K>(&mut self, db: &mut DbGroup<H, K>) -> anyhow::Result<()>
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
        K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
    {
        if let PrunerSchedule::OnceAtStartup {
            versions_to_keep,
            max_batch_size,
            compact_after,
        } = self.schedule
        {
            tracing::info!(
                versions_to_keep,
                compact_after,
                "Running one-time startup pruning to completion"
            );
            self.prune_to_completion(db, versions_to_keep, max_batch_size)?;
            if compact_after {
                db.compact_pruned_cfs()?;
            }
        }
        Ok(())
    }

    /// Drives periodic pruning from `finalize`: commits the in-flight pruner job if it has
    /// finished, then (for [`PrunerConfig::Periodic`]) spawns the next one if enough blocks have
    /// elapsed. Returns the time spent committing a pruning batch this call, for metrics.
    pub(crate) fn on_finalize<H, K>(
        &mut self,
        db: &mut DbGroup<H, K>,
        height: u64,
    ) -> anyhow::Result<Option<Duration>>
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
        K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
    {
        let is_pruner_ready = self
            .in_flight
            .as_ref()
            .map(|p| p.is_finished())
            .unwrap_or(false);
        let mut pruning_commit_time = None;

        if is_pruner_ready {
            // UNWRAP: Checked above.
            let job = self.in_flight.take().unwrap();
            // The pruner thread has already finished (`is_pruner_ready`), so the join is
            // effectively free and this measures the commit.
            let start = std::time::Instant::now();
            let hit_size_limit = self.join_and_commit(db, job)?;
            pruning_commit_time = Some(start.elapsed());
            // If the pruner didn't hit the size limit, we're done. Mark that the pruner finished at
            // the current height. Otherwise, leave the run unmarked so it spawns another iteration.
            if !hit_size_limit {
                self.last_finish_at_height = Some(height);
            }
        }

        if let PrunerSchedule::Periodic {
            block_interval,
            versions_to_keep,
            max_batch_size,
        } = self.schedule
        {
            let should_run_pruner = self.in_flight.is_none()
                && self
                    .last_finish_at_height
                    .map(|last_run_at_height| {
                        height.saturating_sub(last_run_at_height) > block_interval
                    })
                    .unwrap_or(true);
            if should_run_pruner {
                self.in_flight = Some(db.start_pruner(versions_to_keep, max_batch_size));
            }
        }

        Ok(pruning_commit_time)
    }

    /// Prunes synchronously to completion: repeatedly spawn a pruner, immediately join it, and
    /// commit its delete batch, looping while the batch hit the size limit. Reuses the periodic
    /// machinery ([`DbGroup::start_pruner`] / [`Self::join_and_commit`]); joining immediately is
    /// fine here because there is no concurrent finalize traffic during the startup prune.
    fn prune_to_completion<H, K>(
        &mut self,
        db: &mut DbGroup<H, K>,
        versions_to_keep: usize,
        max_batch_size: usize,
    ) -> anyhow::Result<()>
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
        K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
    {
        loop {
            let job = db.start_pruner(versions_to_keep, max_batch_size);
            if !self.join_and_commit(db, job)? {
                break;
            }
        }
        Ok(())
    }

    /// Joins a pruner job, commits its delete batch, updates the test-only counters, and returns
    /// whether the batch hit the size limit (i.e. another pass is needed). Shared by the
    /// synchronous startup prune ([`Self::prune_to_completion`]) and the periodic finalize path.
    fn join_and_commit<H, K>(
        &mut self,
        db: &mut DbGroup<H, K>,
        job: PrunerJob,
    ) -> anyhow::Result<bool>
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
        K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
    {
        let prune_group = job.join()?;
        let hit_size_limit = prune_group.hit_size_limit();
        db.commit_pruning(prune_group)?;
        #[cfg(any(test, feature = "test-utils"))]
        {
            self.commits_count += 1;
            self.last_hit_size_limit = hit_size_limit;
        }
        Ok(hit_size_limit)
    }

    /// Number of pruning batches committed so far. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn commits_count(&self) -> u64 {
        self.commits_count
    }

    /// Whether the most recently committed pruning batch hit the size limit. Test-only
    /// observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn last_hit_size_limit(&self) -> bool {
        self.last_hit_size_limit
    }

    /// Whether a pruner thread is currently in flight. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn is_running(&self) -> bool {
        self.in_flight.is_some()
    }

    /// `None` if no pruner job is in flight, otherwise `Some(finished)` indicating whether that
    /// job's threads have completed. Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn in_flight_finished(&self) -> Option<bool> {
        self.in_flight.as_ref().map(|job| job.is_finished())
    }

    /// Blocks the current thread until any in-flight pruner thread has finished. Does NOT commit
    /// the resulting batch (that happens on the next `finalize`). Test-only observability hook.
    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn wait_for_finish(&self) {
        if let Some(job) = self.in_flight.as_ref() {
            while !job.is_finished() {
                std::thread::yield_now();
            }
        }
    }
}
