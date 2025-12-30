use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Context;
use nomt::hasher::BinaryHasher;
use nomt::{Nomt, Overlay, SessionParams, WitnessMode};
pub use rockbound::versioned_db::HistoricalValueError;
use sov_rollup_interface::reexports::digest;

use super::commit_flag::{CommitFlag, CommitStatus};
use crate::config::RollupDbConfig;
use crate::metrics::nomt::{MerklizedCommitMetric, NomtBeginSessionMetric, NomtDbMetric};

const KERNEL: &str = "kernel_state";
const USER: &str = "user_state";
const BOTH: &str = "user_and_kernel_state";

/// Contains all the most recent rollup data.
pub struct NomtStateDb<H> {
    user: Nomt<BinaryHasher<H>>,
    kernel: Nomt<BinaryHasher<H>>,
}

impl<H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync> NomtStateDb<H> {
    /// Initialize a new [` NomtStateDb `] in the given path.
    pub fn new(config: RollupDbConfig) -> anyhow::Result<Self> {
        tracing::debug!(options = ?config, "Opening NOMT");

        let kernel = {
            let opts = config.get_kernel_options();
            Nomt::<BinaryHasher<H>>::open(opts)?
        };

        let user = {
            let opts = config.get_user_options();
            Nomt::<BinaryHasher<H>>::open(opts)?
        };

        Ok(Self { user, kernel })
    }

    pub(crate) fn validate_commit_flag_and_rollback_if_necssesary(
        &self,
        commit_flag: &CommitFlag,
    ) -> anyhow::Result<()> {
        let commit_status = commit_flag.read_status()?;
        match commit_status {
            CommitStatus::CommittingKernelNomt(saved_hash) => {
                let current_kernel_root_hash = self.kernel.root().into_inner();

                // Kernel commit was sucefull but later commits failed. We rollback only the kernel.
                if saved_hash != current_kernel_root_hash {
                    tracing::warn!(
                        flag_kernel_root_hash = hex::encode(saved_hash),
                        db_kernel_root_hash = hex::encode(current_kernel_root_hash),
                        "Detected in-progress commit {commit_status:?}. Rolling back kernel DB."
                    );

                    self.kernel.rollback(1)?;
                }
            }
            CommitStatus::CommittingUserNomt(saved_hash) => {
                let current_user_root_hash = self.user.root().into_inner();

                // User & Kernel commit was sucefull but later commits failed. We rollback both.
                if saved_hash != current_user_root_hash {
                    tracing::warn!(
                        flag_user_root_hash = hex::encode(saved_hash),
                        db_user_root_hash = hex::encode(current_user_root_hash),
                        "Detected in-progress commit {commit_status:?}. Rolling back kernel DB."
                    );
                    self.kernel.rollback(1)?;
                    self.user.rollback(1)?;
                } else
                // Only Kernel commit was sucesfull. We rollback only the kernel.
                {
                    tracing::warn!(
                      "Detected in-progress commit {commit_status:?}. Rolling back kernel & user DBs."
                    );
                    self.kernel.rollback(1)?;
                }
            }

            CommitStatus::CommittingArchivalUserAndKernel
            | CommitStatus::CommittingLiveUserAndKernel => {
                tracing::warn!(
                "Detected in-progress commit {commit_status:?}. Rolling back kernel & user DBs."
            );
                // User & Kernel commit was sucefull but we don't see `Success`. We rollback both User & Kernek.
                self.kernel.rollback(1)?;
                self.user.rollback(1)?;
            }
            CommitStatus::Success => {}
        }

        Ok(())
    }

    /// Commit [`StateOverlay`] to disk.
    #[tracing::instrument(skip_all)]
    pub(crate) fn commit(
        &self,
        overlay: StateOverlay,
        commit_flag: &CommitFlag,
    ) -> anyhow::Result<MerklizedCommitMetric> {
        let start = std::time::Instant::now();
        let StateOverlay { user, kernel } = overlay;
        // Status should be completed before committing.
        let flag_prepare_start = std::time::Instant::now();
        let flag_prepare = flag_prepare_start.elapsed();

        // 1.
        let flag_mid_start = std::time::Instant::now();
        commit_flag.save_commit_status(&CommitStatus::CommittingKernelNomt(
            self.kernel.root().into_inner(),
        ))?;
        let flag_mid = flag_mid_start.elapsed();

        // 2.
        let write_kernel = self.commit_kernel(kernel)?;

        // 3.
        let flag_finish_start = std::time::Instant::now();

        commit_flag.save_commit_status(&CommitStatus::CommittingUserNomt(
            self.user.root().into_inner(),
        ))?;
        let flag_finish = flag_finish_start.elapsed();

        // 4.
        let write_user = self.commit_user(user)?;

        let total = start.elapsed();
        Ok(MerklizedCommitMetric {
            flag_prepare,
            write_attempts_kernel: 1,
            write_kernel,
            flag_mid,
            write_attempts_user: 1,
            write_user,
            flag_finish,
            total,
        })
    }

    fn commit_kernel(&self, kernel: Overlay) -> anyhow::Result<Duration> {
        let start_kernel = std::time::Instant::now();
        {
            let _span = tracing::debug_span!("namespace_commit", namespace = "kernel").entered();
            kernel
                .commit(&self.kernel)
                .context("kernel namespace commit")?;
        };
        let write_kernel = start_kernel.elapsed();
        Ok(write_kernel)
    }

    fn commit_user(&self, user: Overlay) -> anyhow::Result<Duration> {
        let start_user = std::time::Instant::now();
        {
            let _span = tracing::debug_span!("namespace_commit", namespace = "user").entered();
            user.commit(&self.user).context("user namespace commit")?;
        };
        let write_user = start_user.elapsed();
        Ok(write_user)
    }

    /// Commit [`crate::storage_manager::StateFinishedSession`] to disk.
    #[cfg(feature = "test-utils")]
    pub fn commit_change_set(
        &self,
        session: crate::storage_manager::StateFinishedSession,
        commit_flag: &CommitFlag,
    ) -> anyhow::Result<MerklizedCommitMetric> {
        let overlay = session.into_state_overlay();
        self.commit(overlay, commit_flag)
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
    /// Returned session does not collect witness!
    /// Use `Self::begin_both_sessions` if witness is needed
    #[tracing::instrument(skip(self))]
    pub fn begin_user_session_without_witness(
        &self,
    ) -> anyhow::Result<nomt::Session<BinaryHasher<H>>> {
        let start = std::time::Instant::now();
        let mut overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
        let mut overlays_count = 0;
        let session = {
            let snapshots = self.all_snapshots.read().expect("Snapshots lock poisoned");
            for overlay_ref in &self.relevant_snapshot_refs {
                let Some(state_overlay) = snapshots.get(overlay_ref) else {
                    tracing::debug!(
                        "Cannot find snapshot from reference, assuming it has been committed"
                    );
                    continue;
                };
                overlays.push(&state_overlay.user);
                overlays_count += 1;
            }
            let params = SessionParams::default().overlay(overlays).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to construct session params for user session: {:?}",
                    e
                )
            })?;
            self.state_db.user.begin_session(params)
        };
        let init_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: USER,
                overlays: overlays_count,
                init_time,
            });
        });
        Ok(session)
    }

    /// Build [`nomt::Session`] for [`crate::namespaces::KernelNamespace`].
    /// Should be called only when really needed.
    /// Will hold the read lock to all snapshots.
    /// **Commiting storage will be blocked until all built sessions are deallocated.**
    /// Returned session does not collect witness!
    /// Use `Self::begin_both_sessions` if witness is needed
    #[tracing::instrument(skip(self))]
    pub fn begin_kernel_session_without_witness(
        &self,
    ) -> anyhow::Result<nomt::Session<BinaryHasher<H>>> {
        let start = std::time::Instant::now();
        let mut overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
        let mut overlays_count = 0;
        let session = {
            let snapshots = self.all_snapshots.read().expect("Snapshots lock poisoned");
            for overlay_ref in &self.relevant_snapshot_refs {
                let Some(state_overlay) = snapshots.get(overlay_ref) else {
                    tracing::debug!(
                        "Cannot find snapshot from reference, assuming it has been committed"
                    );
                    continue;
                };
                overlays.push(&state_overlay.kernel);
                overlays_count += 1;
            }
            let params = SessionParams::default().overlay(overlays).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to construct session params for kernel session: {:?}",
                    e
                )
            })?;
            self.state_db.kernel.begin_session(params)
        };
        let init_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: KERNEL,
                overlays: overlays_count,
                init_time,
            });
        });
        Ok(session)
    }

    /// Begins both sessions at the same time.
    /// Should be used if both sessions are needed in same context. Prevents dead lock.
    pub fn begin_both_sessions(&self, with_witness: bool) -> anyhow::Result<SessionsContainer<H>> {
        let start = std::time::Instant::now();
        let mut kernel_overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
        let mut user_overlays = Vec::with_capacity(self.relevant_snapshot_refs.len());
        let mut overlays_count = 0;
        let (kernel_session, user_session) = {
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
                overlays_count += 1;
            }
            let mut kernel_params =
                SessionParams::default()
                    .overlay(kernel_overlays)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to construct session params for kernel session: {:?}",
                            e
                        )
                    })?;
            if with_witness {
                kernel_params = kernel_params.witness_mode(WitnessMode::read_write());
            }
            let mut user_params = SessionParams::default()
                .overlay(user_overlays)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to construct session params for user session: {:?}",
                        e
                    )
                })?;
            if with_witness {
                user_params = user_params.witness_mode(WitnessMode::read_write());
            }

            let kernel_session = self.state_db.kernel.begin_session(kernel_params);
            let user_session = self.state_db.user.begin_session(user_params);
            (kernel_session, user_session)
        };
        let init_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtBeginSessionMetric {
                db: BOTH,
                overlays: overlays_count,
                init_time,
            });
        });

        Ok(SessionsContainer {
            user: user_session,
            kernel: kernel_session,
        })
    }
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
        let commit_flag = CommitFlag::new(&config.path);
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

            let user_session = builder.begin_user_session_without_witness().unwrap();
            let finished_user_session = user_session.finish(writes.clone()).unwrap();

            let kernel_session = builder.begin_kernel_session_without_witness().unwrap();

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
            let user_session = check_builder.begin_user_session_without_witness().unwrap();
            let kernel_session = check_builder
                .begin_kernel_session_without_witness()
                .unwrap();
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
            state_db.commit(overlay, &commit_flag).unwrap();
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
        let commit_flag = CommitFlag::new(&config.path);
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());
        state_db
            .validate_commit_flag_and_rollback_if_necssesary(&commit_flag)
            .unwrap();

        let all_overlays: HashMap<u64, StateOverlay> = HashMap::new();
        let all_overlays = Arc::new(RwLock::new(all_overlays));

        // Populate the state db with some data and commit it immediately.
        {
            let builder = NomtSessionBuilder::<H, u64>::new(
                state_db.clone(),
                Vec::new(),
                all_overlays.clone(),
            );

            let user_session = builder.begin_user_session_without_witness().unwrap();
            let kernel_session = builder.begin_kernel_session_without_witness().unwrap();

            let finished_user_session = user_session.finish(initial_user_writes).unwrap();
            let finished_kernel_session = kernel_session.finish(initial_kernel_writes).unwrap();
            let overlay = StateFinishedSession::new(finished_user_session, finished_kernel_session)
                .into_state_overlay();
            state_db.commit(overlay, &commit_flag).unwrap();
        }

        // Base overlays
        let base_builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), Vec::new(), all_overlays.clone());

        let user_session = base_builder.begin_user_session_without_witness().unwrap();
        let kernel_session = base_builder.begin_kernel_session_without_witness().unwrap();
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

        let user_session = test_builder.begin_user_session_without_witness().unwrap();
        let kernel_session = test_builder.begin_kernel_session_without_witness().unwrap();
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
            user: _,
            kernel: base_kernel_overlay,
        } = base_overlay;

        base_kernel_overlay.commit(&state_db.kernel).unwrap();

        let test_commit_result = state_db.commit(test_overlay, &commit_flag);
        assert!(test_commit_result.is_err());

        // Reopen the state db.
        drop(state_db);
        let config = RollupDbConfig::default_in_path(temp_dir.path().to_path_buf());
        let state_db = Arc::new(NomtStateDb::<H>::new(config).unwrap());
        state_db
            .validate_commit_flag_and_rollback_if_necssesary(&commit_flag)
            .unwrap();

        let builder =
            NomtSessionBuilder::<H, u64>::new(state_db.clone(), Vec::new(), all_overlays.clone());

        let user_session = builder.begin_user_session_without_witness().unwrap();
        let kernel_session = builder.begin_kernel_session_without_witness().unwrap();

        let user_value = user_session.read(user_key_path).unwrap();
        let kernel_value = kernel_session.read(kernel_key_path).unwrap();

        assert_eq!(kernel_value, Some(value_2.clone()));
        assert_eq!(user_value, Some(value_1.clone()));
    }
}
