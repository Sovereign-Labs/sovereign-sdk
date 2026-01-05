use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::RollupDbConfig;
use crate::metrics::nomt::{MerklizedCommitMetric, NomtBeginSessionMetric, NomtDbMetric};
use anyhow::Context;
use nomt::hasher::BinaryHasher;
use nomt::{Nomt, Overlay, SessionParams, WitnessMode};
pub use rockbound::versioned_db::HistoricalValueError;
use sov_rollup_interface::reexports::digest;

const KERNEL: &str = "kernel_state";
const USER: &str = "user_state";
const BOTH: &str = "user_and_kernel_state";

/// Contains all the most recent rollup data.
pub struct NomtStateDb<H> {
    pub(crate) user: Nomt<BinaryHasher<H>>,
    pub(crate) kernel: Nomt<BinaryHasher<H>>,
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

    /// Commit [`StateOverlay`] to disk.
    #[tracing::instrument(skip_all)]
    pub(crate) fn commit(&self, overlay: StateOverlay) -> anyhow::Result<MerklizedCommitMetric> {
        let start = std::time::Instant::now();
        let StateOverlay { user, kernel } = overlay;
        // Status should be completed before committing.
        let flag_prepare_start = std::time::Instant::now();
        let flag_prepare = flag_prepare_start.elapsed();

        // 1.
        let flag_mid_start = std::time::Instant::now();
        let flag_mid = flag_mid_start.elapsed();

        #[cfg(feature = "test-utils")]
        crate::test_utils::CrashLocation::BeforeCommittingKernelNomt.crash_if_env_set();

        // 2.
        let write_kernel = self.commit_kernel(kernel)?;

        // 3.
        let flag_finish_start = std::time::Instant::now();
        let flag_finish = flag_finish_start.elapsed();

        #[cfg(feature = "test-utils")]
        crate::test_utils::CrashLocation::BeforeCommittingUserNomt.crash_if_env_set();

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
    ) -> anyhow::Result<MerklizedCommitMetric> {
        let overlay = session.into_state_overlay();
        self.commit(overlay)
    }

    pub(crate) fn get_root_hashes(&self) -> StateRootHashes {
        StateRootHashes {
            user: self.user.root().into_inner(),
            kernel: self.kernel.root().into_inner(),
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
    pub(crate) user: [u8; 32],
    pub(crate) kernel: [u8; 32],
}

impl StateRootHashes {
    pub(crate) fn is_empty(&self) -> bool {
        self.user == nomt::trie::TERMINATOR && self.kernel == nomt::trie::TERMINATOR
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
            state_db.commit(overlay).unwrap();
        }
    }
}
