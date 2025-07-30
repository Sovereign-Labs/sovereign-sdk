use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use nomt::hasher::BinaryHasher;
use nomt::{Nomt, Overlay, SessionParams, WitnessMode};
use sov_rollup_interface::reexports::digest;

use super::commit_flag::{CommitFlag, CommitStatus};
use crate::config::RollupDbConfig;
use crate::metrics::nomt::{NomtBeginSessionMetric, NomtDbMetric};

const KERNEL: &str = "kernel_state";
const USER: &str = "user_state";
const BOTH: &str = "user_and_kernel_state";

const COMMIT_START_DELAY: std::time::Duration = std::time::Duration::from_millis(1);
const COMMIT_RETRY_ATTEMPTS: usize = 26;

/// Contains all the most recent rollup data.
pub struct NomtStateDb<H> {
    user: Nomt<BinaryHasher<H>>,
    kernel: Nomt<BinaryHasher<H>>,
    commit_flag: CommitFlag,
}

impl<H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync> NomtStateDb<H> {
    /// Initialize a new [` NomtStateDb `] in the given path.
    pub fn new(config: RollupDbConfig) -> anyhow::Result<Self> {
        let commit_flag = CommitFlag::new(&config.path);

        tracing::debug!(options = ?config, "Opening NOMT");

        let kernel = {
            let opts = config.get_kernel_options();
            Nomt::<BinaryHasher<H>>::open(opts)?
        };

        match commit_flag.read_status()? {
            CommitStatus::InProgress(detected_root_hash) => {
                let current_kernel_root_hash = kernel.root().into_inner();
                tracing::warn!(
                    flag_kernel_root_hash = hex::encode(detected_root_hash),
                    db_kernel_root_hash = hex::encode(current_kernel_root_hash),
                    "Detected in-progress commit. Rolling back kernel DB."
                );
                if current_kernel_root_hash != detected_root_hash {
                    anyhow::bail!("Unsafe to perform rollback, status root hash {} does not match database {}. Manual intervention is needed",
                        hex::encode(detected_root_hash),
                        hex::encode(current_kernel_root_hash),
                    );
                }
                kernel
                    .rollback(1)
                    .context("Failed to rollback kernel DB after in-progress commit detected")?;
                commit_flag
                    .write_status(CommitStatus::Completed)
                    .with_context(|| {
                        commit_flag.log_reset_instruction();
                        "Failed to write `COMPLETED` status after rollback. Manual intervention required."
                    })?;
                let rolled_back_root_hash = kernel.root().into_inner();
                tracing::info!(
                    from_root_hash = hex::encode(current_kernel_root_hash),
                    to_root_hash = hex::encode(rolled_back_root_hash),
                    "Kernel namespace rollback completed"
                );
            }
            CommitStatus::Completed => {
                tracing::trace!("Commit flag is `Completed`, proceeding as usual");
            }
        }

        let user = {
            let opts = config.get_user_options();
            Nomt::<BinaryHasher<H>>::open(opts)?
        };

        Ok(Self {
            user,
            kernel,
            commit_flag,
        })
    }

    /// Commit [`StateOverlay`] to disk.
    #[tracing::instrument(skip_all)]
    pub(crate) fn commit(&self, overlay: StateOverlay) -> anyhow::Result<()> {
        let StateOverlay { user, kernel } = overlay;
        // Status should be completed before committing.
        debug_assert_eq!(self.commit_flag.read_status()?, CommitStatus::Completed);

        let in_progress_commit_status = CommitStatus::InProgress(kernel.root().into_inner());

        {
            let _span = tracing::debug_span!("namespace_commit", namespace = "kernel").entered();
            try_commit_overlay_with_backoff(&self.kernel, kernel)
                .context("kernel namespace commit")?;
        }

        // If the kernel commit fails, the flag is untouched, meaning DB remains synced on previous state.
        // Write IN-PROGRESS status after kernel committed successfully.

        // 2. Kernel commit succeeded. Try to set flag to IN-PROGRESS.
        if let Err(flag_write_err) = self.commit_flag.write_status(in_progress_commit_status) {
            // CRITICAL: Kernel committed, but couldn't write IN-PROGRESS flag.
            // Try to roll back the kernel commit to revert to a consistent state.
            // Failures here are more likely to be due to persistent disk issues (full, I/O errors, permissions).
            tracing::error!(
                error = ?flag_write_err,
                "Kernel commit succeeded, but failed to write IN-PROGRESS flag: Attempting kernel rollback.",
            );
            if let Err(rollback_err) = self.kernel.rollback(1) {
                // DISASTER: Kernel committed, flag is still COMPLETED, and kernel rollback FAILED.
                // The database is in an inconsistent state that cannot be automatically recovered by this logic.
                // Propagate a combined error. This situation likely requires manual intervention or node reset.
                return Err(anyhow::anyhow!(
                "CRITICAL INCONSISTENCY: Kernel committed, but failed to write IN-PROGRESS flag ({:?}), \
                 AND subsequent kernel rollback also failed ({:?}). Manual intervention required.",
                flag_write_err,
                rollback_err
            ));
            }
            // Kernel rollback succeeded.
            // The DB is back to its state before this commit attempt.
            // Return an error indicating the flag write failure, but state is consistent.
            tracing::warn!("Kernel rollback succeeded after IN-PROGRESS flag write failure. DB state is consistent with previous version, but flag does not match. Manual intervention required.");
            return Err(flag_write_err).with_context(|| {
                self.commit_flag.log_reset_instruction();
                "Failed to write IN-PROGRESS status after kernel commit; kernel was rolled back, but flag does not match. Manual intervention required."
            });
        }

        debug_assert_eq!(self.commit_flag.read_status()?, in_progress_commit_status);

        {
            let _span = tracing::debug_span!("namespace_commit", namespace = "user").entered();
            try_commit_overlay_with_backoff(&self.user, user).context("user namespace commit")?;
        }

        self.commit_flag
            .write_status(CommitStatus::Completed)
            .with_context(|| {
                self.commit_flag.log_reset_instruction();
                "Failed to write `COMPLETED` status after successful user commit"
            })?;
        debug_assert_eq!(self.commit_flag.read_status()?, CommitStatus::Completed);
        Ok(())
    }

    /// Commit [`crate::storage_manager::StateFinishedSession`] to disk.
    #[cfg(feature = "test-utils")]
    pub fn commit_change_set(
        &self,
        session: crate::storage_manager::StateFinishedSession,
    ) -> anyhow::Result<()> {
        let overlay = session.into_state_overlay();
        self.commit(overlay)
    }

    pub(crate) fn full_rollback(&self) -> anyhow::Result<()> {
        self.user.rollback(1)?;
        self.kernel.rollback(1)?;
        Ok(())
    }

    pub(crate) fn get_root_hashes(&self) -> StateRootHashes {
        StateRootHashes {
            user: self.user.root(),
            kernel: self.kernel.root(),
        }
    }

    pub(crate) fn send_metrics(&self) {
        let user_metrics = NomtDbMetric::new(USER, &self.user);
        let kernel_metrics = NomtDbMetric::new(KERNEL, &self.kernel);
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(user_metrics);
            tracker.submit(kernel_metrics);
        });
    }
}

#[derive(Debug)]
pub(crate) struct StateRootHashes {
    pub(crate) user: nomt::Root,
    pub(crate) kernel: nomt::Root,
}

impl StateRootHashes {
    // It is known that historical root hash is a concatenation of 2 root hashes,
    // but we don't want to duplicate logic between `sov_state` and here.
    // So just the simple inclusion of 2 root hashes is enough.
    // This is based on an assumption that serialized root hash is stored as is, without any permutation.
    pub(crate) fn included_in_raw(
        &self,
        combined_historical_root: &rockbound::SchemaValue,
    ) -> bool {
        if combined_historical_root.len() < 64 {
            tracing::warn!(
                "Combined historical root is too short to contain 2 root hashes: {}",
                combined_historical_root.len()
            );
            return false;
        }
        let user_needle: &[u8] = self.user.as_ref();
        let kernel_needle: &[u8] = self.kernel.as_ref();
        assert_eq!(user_needle.len(), 32);
        assert_eq!(kernel_needle.len(), 32);

        let user_found = combined_historical_root
            .windows(32)
            .any(|window| window == user_needle);
        let kernel_found = combined_historical_root
            .windows(32)
            .any(|window| window == kernel_needle);
        user_found && kernel_found
    }
}

/// Combination of [`Overlay`] for user and kernel namespaces.
pub(crate) struct StateOverlay {
    pub(crate) user: Overlay,
    pub(crate) kernel: Overlay,
}

/// Container of all necessary information to build a [`nomt::Session`] for a given namespace.
pub struct NomtSessionBuilder<H, K> {
    state_db: Arc<NomtStateDb<H>>,
    // In revered chronological order, as [`nomt::SessionParams::overlay`] expects
    relevant_snapshot_refs: Vec<K>,
    // Reference to snapshots for all blocks currently available in a storage manager.
    // Used to get reference to state overlays and build session.
    all_snapshots: Arc<RwLock<HashMap<K, StateOverlay>>>,
}

impl<H, K: Clone> Clone for NomtSessionBuilder<H, K> {
    fn clone(&self) -> Self {
        Self {
            state_db: self.state_db.clone(),
            relevant_snapshot_refs: self.relevant_snapshot_refs.clone(),
            all_snapshots: self.all_snapshots.clone(),
        }
    }
}

impl<H, K> NomtSessionBuilder<H, K> {
    /// Parameters:
    ///  * `state_db` - Reference to [`NomtStateDb`].
    ///  * `relevant_snapshot_refs`: In revered chronological order, as [`SessionParams::overlay`] expects
    ///  * `all_snapshots`. Should be the same structure that is used by a storage manager
    pub(crate) fn new(
        state_db: Arc<NomtStateDb<H>>,
        relevant_snapshot_refs: Vec<K>,
        all_snapshots: Arc<RwLock<HashMap<K, StateOverlay>>>,
    ) -> Self {
        Self {
            state_db,
            relevant_snapshot_refs,
            all_snapshots,
        }
    }
}

/// Container for both sessions, to remove error in passing them
pub struct SessionsContainer<H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync> {
    #[allow(missing_docs)]
    pub user: nomt::Session<BinaryHasher<H>>,
    #[allow(missing_docs)]
    pub kernel: nomt::Session<BinaryHasher<H>>,
}

impl<H, K> NomtSessionBuilder<H, K>
where
    K: Eq + std::hash::Hash,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
{
    /// Build [`nomt::Session`] for [`crate::namespaces::UserNamespace`].
    /// Should be called only when really needed.
    /// Will hold the read lock to all snapshots.
    /// **Commiting storage will be blocked until all built sessions are deallocated.**
    #[tracing::instrument(skip(self))]
    pub fn begin_user_session(&self) -> anyhow::Result<nomt::Session<BinaryHasher<H>>> {
        let start = std::time::Instant::now();
        let params = {
            let mut overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
            let snapshots = self.all_snapshots.read().expect("Snapshots lock poisoned");
            for overlay_ref in &self.relevant_snapshot_refs {
                let Some(state_overlay) = snapshots.get(overlay_ref) else {
                    tracing::debug!(
                        "Cannot find snapshot from reference, assuming it has been committed"
                    );
                    continue;
                };
                overlays.push(&state_overlay.user);
            }
            SessionParams::default()
                .overlay(overlays)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to construct session params for user session: {:?}",
                        e
                    )
                })?
                .witness_mode(WitnessMode::read_write())
        };
        let session = self.state_db.user.begin_session(params);
        let init_time = start.elapsed();
        let overlays = self.relevant_snapshot_refs.len();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: USER,
                overlays,
                init_time,
            });
        });
        Ok(session)
    }

    /// Build [`nomt::Session`] for [`crate::namespaces::KernelNamespace`].
    /// Should be called only when really needed.
    /// Will hold the read lock to all snapshots.
    /// **Commiting storage will be blocked until all built sessions are deallocated.**
    #[tracing::instrument(skip(self))]
    pub fn begin_kernel_session(&self) -> anyhow::Result<nomt::Session<BinaryHasher<H>>> {
        let start = std::time::Instant::now();
        let params = {
            let mut overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
            let snapshots = self.all_snapshots.read().expect("Snapshots lock poisoned");
            for overlay_ref in &self.relevant_snapshot_refs {
                let Some(state_overlay) = snapshots.get(overlay_ref) else {
                    tracing::debug!(
                        "Cannot find snapshot from reference, assuming it has been committed"
                    );
                    continue;
                };
                overlays.push(&state_overlay.kernel);
            }
            SessionParams::default()
                .overlay(overlays)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to construct session params for kernel session: {:?}",
                        e
                    )
                })?
                .witness_mode(WitnessMode::read_write())
        };
        let session = self.state_db.kernel.begin_session(params);
        let init_time = start.elapsed();
        let overlays = self.relevant_snapshot_refs.len();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: KERNEL,
                overlays,
                init_time,
            });
        });
        Ok(session)
    }

    /// Begins both sessions at the same time.
    /// Should be used if both sessions are needed in same context. Prevents dead lock.
    pub fn begin_both_sessions(&self) -> anyhow::Result<SessionsContainer<H>> {
        let start = std::time::Instant::now();
        let (kernel_params, user_params) = {
            let mut kernel_overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
            let mut user_overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
            let snapshots = self.all_snapshots.read().expect("Snapshots lock poisoned");
            for overlay_ref in &self.relevant_snapshot_refs {
                let Some(state_overlay) = snapshots.get(overlay_ref) else {
                    tracing::debug!(
                        "Cannot find snapshot from reference, assuming it has been committed"
                    );
                    continue;
                };
                kernel_overlays.push(&state_overlay.kernel);
                user_overlays.push(&state_overlay.user);
            }
            let kernel_params = SessionParams::default()
                .overlay(kernel_overlays)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to construct session params for kernel session: {:?}",
                        e
                    )
                })?
                .witness_mode(WitnessMode::read_write());
            let user_params = SessionParams::default()
                .overlay(user_overlays)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to construct session params for user session: {:?}",
                        e
                    )
                })?
                .witness_mode(WitnessMode::read_write());
            (kernel_params, user_params)
        };
        let kernel_session = self.state_db.kernel.begin_session(kernel_params);
        let user_session = self.state_db.user.begin_session(user_params);
        let init_time = start.elapsed();
        let overlays = self.relevant_snapshot_refs.len();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: BOTH,
                overlays,
                init_time,
            });
        });

        Ok(SessionsContainer {
            user: user_session,
            kernel: kernel_session,
        })
    }
}

/// An attempt to commit an overlay to the given `nomt` instance.
///
/// This function will retry the commit with an exponential backoff if it fails
/// due to contention.
/// This is necessary because another thread might be holding a lock on the NOMT.
/// The function will attempt to commit a total of [`COMMIT_RETRY_ATTEMPTS`] times before giving up and returning an error.
fn try_commit_overlay_with_backoff<H>(
    nomt: &Nomt<BinaryHasher<H>>,
    mut overlay: Overlay,
) -> anyhow::Result<()>
where
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
{
    let mut current_wait = COMMIT_START_DELAY;
    for attempt in 0..COMMIT_RETRY_ATTEMPTS {
        match overlay.try_commit_nonblocking(nomt)? {
            None => {
                tracing::trace!(attempts = %attempt, "Commit completed");
                return Ok(());
            }
            Some(returned) => {
                match attempt {
                    n if n > 20 => {
                        tracing::warn!(%attempt, wait_time = ?current_wait, "Failed to commit overlay, retrying...");
                    }
                    n if n > 10 => {
                        tracing::info!(%attempt, wait_time = ?current_wait, "Failed to commit overlay, retrying...");
                    }
                    _ => {
                        tracing::debug!(%attempt, wait_time = ?current_wait, "Failed to commit overlay, retrying...");
                    }
                };
                overlay = returned;
                std::thread::sleep(current_wait);
                // Apply exponential backoff with factor 1.5:
                // multiply by 3 then divide by 2 to get 1.5x
                // Use saturating operations to prevent overflow
                let next_nanos = current_wait.as_nanos().saturating_mul(3).saturating_div(2);

                current_wait = std::time::Duration::from_nanos(
                    next_nanos
                        .try_into()
                        .expect("Nanos overflow for NOMT commit retry"),
                );
            }
        }
    }

    anyhow::bail!(
        "Failed to commit overlay after {} attempts",
        COMMIT_RETRY_ATTEMPTS
    );
}

/// Begin a new user and kernel session with only data that has been written to disk
#[cfg(feature = "test-utils")]
pub fn get_session_builder_from_committed<H, K>(
    state_db: Arc<NomtStateDb<H>>,
) -> NomtSessionBuilder<H, K>
where
    K: Clone + Eq + std::hash::Hash,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
{
    let empty_snapshots = Arc::new(RwLock::new(HashMap::new()));
    NomtSessionBuilder::new(state_db, Vec::new(), empty_snapshots)
}

#[cfg(test)]
mod tests {
    use nomt::trie::KeyPath;
    use sha2::Digest;

    use super::*;
    use crate::storage_manager::StateFinishedSession;
    use crate::test_utils::H;

    fn from_key(key: u64) -> (KeyPath, Option<Vec<u8>>) {
        let raw_data = key.to_be_bytes();
        let key_path: KeyPath = H::digest(raw_data).into();
        (key_path, Some(raw_data.to_vec()))
    }

    #[test]
    fn test_session_can_be_built_while_finalized() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = RollupDbConfig::default_in_path(temp_dir.path().to_path_buf());
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());

        // First produce some overlays with data
        let all_overlays: HashMap<u64, StateOverlay> = HashMap::new();
        let all_overlays = Arc::new(RwLock::new(all_overlays));
        let rounds = 10;
        for this_ref in 0..rounds {
            let mut overlay_refs = (0..this_ref).collect::<Vec<_>>();
            overlay_refs.reverse();
            let builder = NomtSessionBuilder::<H, u64>::new(
                state_db.clone(),
                overlay_refs,
                all_overlays.clone(),
            );

            let (key_path, data) = from_key(this_ref);
            let writes = vec![(key_path, nomt::KeyReadWrite::Write(data))];

            let user_session = builder.begin_user_session().unwrap();
            let finished_user_session = user_session.finish(writes.clone()).unwrap();

            let kernel_session = builder.begin_kernel_session().unwrap();

            let finished_kernel_session = kernel_session.finish(writes.clone()).unwrap();
            let overlay = StateFinishedSession::new(finished_user_session, finished_kernel_session)
                .into_state_overlay();
            let mut overlays = all_overlays.write().unwrap();
            overlays.insert(this_ref, overlay);
        }
        let mut overlay_refs = (0..rounds).collect::<Vec<_>>();
        overlay_refs.reverse();
        let check_builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), overlay_refs, all_overlays.clone());
        for commiting_ref in 0..rounds {
            let user_session = check_builder.begin_user_session().unwrap();
            let kernel_session = check_builder.begin_kernel_session().unwrap();
            for this_ref in 0..rounds {
                let (key_path, expected_value) = from_key(this_ref);
                let user_value = user_session.read(key_path).unwrap();
                let kernel_value = kernel_session.read(key_path).unwrap();

                assert_eq!(
                    user_value, expected_value,
                    "failed to check value for ref: {this_ref}"
                );
                assert_eq!(kernel_value, expected_value);
            }
            drop(user_session);
            drop(kernel_session);
            let mut overlays = all_overlays.write().unwrap();
            let overlay = overlays.remove(&commiting_ref).unwrap();
            state_db.commit(overlay).unwrap();
        }
    }

    // This test emulates failure behaviour during a commit.
    // We want to test a case when a kernel database commits, but the user fails.
    // First, we write some data to both databases to have a reference point.
    // Then we create 2 overlays, let's call them "base overlays".
    // They have data, but are not committed.
    // Then we create 2 more overlays on top of those 2, let's call them "test overlays".
    // Then the "base" kernel overlay is committed, but the "user" base overlay is kept in memory.
    // This creates a precondition for `StateOverlay::commit` to fail on the user overlay phase.
    // After this error is confirmed, we commit the "base user" overlay and re-open the NOMT database.
    // We expect that both user and kernel databases are in sync and on data from base overlays.
    #[test]
    fn test_namespaced_db_stay_in_sync_on_error() {
        let temp_dir = tempfile::tempdir().unwrap();

        let user_key = b"user_key".to_vec();
        let user_key_path: KeyPath = H::digest(&user_key).into();
        let value_1 = b"value_1".to_vec();
        let value_2 = b"value_2".to_vec();
        let value_3 = b"value_3".to_vec();
        let initial_user_writes = vec![(
            user_key_path,
            nomt::KeyReadWrite::Write(Some(value_1.clone())),
        )];
        let base_user_writes = vec![(
            user_key_path,
            nomt::KeyReadWrite::Write(Some(value_2.clone())),
        )];
        let test_user_writes = vec![(
            user_key_path,
            nomt::KeyReadWrite::Write(Some(value_3.clone())),
        )];

        let kernel_key = b"kernel_key".to_vec();
        let kernel_key_path: KeyPath = H::digest(&kernel_key).into();
        let initial_kernel_writes = vec![(
            kernel_key_path,
            nomt::KeyReadWrite::Write(Some(value_1.clone())),
        )];
        let base_kernel_writes = vec![(
            kernel_key_path,
            nomt::KeyReadWrite::Write(Some(value_2.clone())),
        )];
        let test_kernel_writes = vec![(
            kernel_key_path,
            nomt::KeyReadWrite::Write(Some(value_3.clone())),
        )];

        let config = RollupDbConfig::default_in_path(temp_dir.path().to_path_buf());
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());

        let all_overlays: HashMap<u64, StateOverlay> = HashMap::new();
        let all_overlays = Arc::new(RwLock::new(all_overlays));

        // Populate the state db with some data and commit it immediately.
        {
            let builder = NomtSessionBuilder::<H, u64>::new(
                state_db.clone(),
                Vec::new(),
                all_overlays.clone(),
            );

            let user_session = builder.begin_user_session().unwrap();
            let kernel_session = builder.begin_kernel_session().unwrap();

            let finished_user_session = user_session.finish(initial_user_writes).unwrap();
            let finished_kernel_session = kernel_session.finish(initial_kernel_writes).unwrap();
            let overlay = StateFinishedSession::new(finished_user_session, finished_kernel_session)
                .into_state_overlay();
            state_db.commit(overlay).unwrap();
        }

        // Base overlays
        let base_builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), Vec::new(), all_overlays.clone());

        let user_session = base_builder.begin_user_session().unwrap();
        let kernel_session = base_builder.begin_kernel_session().unwrap();
        drop(base_builder);

        let finished_user_session = user_session.finish(base_user_writes).unwrap();
        let finished_kernel_session = kernel_session.finish(base_kernel_writes).unwrap();
        let base_overlay =
            StateFinishedSession::new(finished_user_session, finished_kernel_session)
                .into_state_overlay();

        {
            let mut overlays = all_overlays.write().unwrap();
            overlays.insert(0, base_overlay);
        }

        let test_builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), vec![0], all_overlays.clone());

        let user_session = test_builder.begin_user_session().unwrap();
        let kernel_session = test_builder.begin_kernel_session().unwrap();
        drop(test_builder);

        let finished_user_session = user_session.finish(test_user_writes).unwrap();
        let finished_kernel_session = kernel_session.finish(test_kernel_writes).unwrap();

        let test_overlay =
            StateFinishedSession::new(finished_user_session, finished_kernel_session)
                .into_state_overlay();

        let base_overlay = {
            let mut overlays = all_overlays.write().unwrap();
            overlays.remove(&0).unwrap()
        };
        let StateOverlay {
            user: base_user_overlay,
            kernel: base_kernel_overlay,
        } = base_overlay;

        base_kernel_overlay.commit(&state_db.kernel).unwrap();

        let test_commit_result = state_db.commit(test_overlay);
        assert!(test_commit_result.is_err());

        base_user_overlay.commit(&state_db.user).unwrap();

        // Reopen the state db.
        drop(state_db);
        let config = RollupDbConfig::default_in_path(temp_dir.path().to_path_buf());
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());

        let builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), Vec::new(), all_overlays.clone());

        let user_session = builder.begin_user_session().unwrap();
        let kernel_session = builder.begin_kernel_session().unwrap();

        let user_value = user_session.read(user_key_path).unwrap();
        let kernel_value = kernel_session.read(kernel_key_path).unwrap();
        assert_eq!(user_value, Some(value_2.clone()));
        assert_eq!(user_value, kernel_value);
    }

    #[test]
    fn test_commit_with_live_user_session_will_eventually_fail() {
        commit_with_live_session_will_eventually_fail(true);
    }

    #[test]
    fn test_commit_with_live_kernel_session_will_eventually_fail() {
        commit_with_live_session_will_eventually_fail(false);
    }

    fn commit_with_live_session_will_eventually_fail(is_user_session: bool) {
        // Setup
        let temp_dir = tempfile::tempdir().unwrap();
        let config = RollupDbConfig::default_in_path(temp_dir.path().to_path_buf());
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());

        let all_overlays: HashMap<u64, StateOverlay> = HashMap::new();
        let all_overlays = Arc::new(RwLock::new(all_overlays));
        let builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), Vec::new(), all_overlays.clone());
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

        // Starting background live session, until shutdown signal is sent
        let background_session = if is_user_session {
            builder.begin_user_session().unwrap()
        } else {
            builder.begin_kernel_session().unwrap()
        };
        let thread_handle = std::thread::spawn(move || {
            let _root = background_session.prev_root();
            shutdown_rx.recv().unwrap();
        });

        // Building some changes
        let user_session = builder.begin_user_session().unwrap();
        let kernel_session = builder.begin_kernel_session().unwrap();

        let key = b"test_key".to_vec();
        let key_path: KeyPath = H::digest(&key).into();
        let value = b"test_value".to_vec();
        let writes = vec![(key_path, nomt::KeyReadWrite::Write(Some(value)))];

        let finished_user = user_session.finish(writes.clone()).unwrap();
        let finished_kernel = kernel_session.finish(writes).unwrap();

        let overlay =
            StateFinishedSession::new(finished_user, finished_kernel).into_state_overlay();

        // Trying to commit
        assert!(state_db.commit(overlay).is_err());

        shutdown_tx.send(()).unwrap();
        thread_handle.join().unwrap();
    }
}
