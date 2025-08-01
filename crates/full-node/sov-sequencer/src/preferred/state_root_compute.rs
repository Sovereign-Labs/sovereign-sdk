//! Defines the logic for computing state roots for the preferred sequencer.

use std::collections::BTreeMap;

use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{Spec, Storage};
use sov_rollup_interface::common::SlotNumber;
use sov_state::{NativeStorage, SlotKey, SlotValue, StateAccesses, StateRoot};
use tokio::sync::{mpsc, oneshot};

/// The memory limit for old write sets that we keep around. These old sets are useful because they tell us what keys have been changed.
/// Which lets us output a much handier error message when the state root computation changes.
const MEMORY_LIMIT_FOR_STATE_ROOT_COMPUTATION: usize = 100_000_000; // 100MB
const MAX_STATE_ROOTS_TO_CACHE: usize = 100;
const NUM_STATE_ROOT_COMPUTE_REQUESTS: usize = 50;

pub(crate) struct StateRootComputeRequest<S: Spec> {
    pub state_accesses: StateAccesses,
    pub storage: S::Storage,
    pub rollup_height: RollupHeight,
    pub max_slot_number: SlotNumber,
    pub response_channel: oneshot::Sender<(RollupHeight, <S::Storage as Storage>::Root)>,
}

struct StateRootCacheEntry<S: Spec> {
    root: <S::Storage as Storage>::Root,
    writes: Option<Vec<(sov_state::SlotKey, Option<SlotValue>)>>,
    size: usize,
}

fn user_roots_match<S: StateRoot>(old_root: &S, new_root: &S) -> bool {
    let old_user_root = old_root.namespace_root(sov_state::ProvableNamespace::User);
    let new_user_root = new_root.namespace_root(sov_state::ProvableNamespace::User);
    old_user_root == new_user_root
}

impl<S: Spec> StateRootCacheEntry<S> {
    // Assert that the state root is consistent with the write set.
    async fn assert_consistency(
        &self,
        state_accesses: StateAccesses,
        storage: S::Storage,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> (RollupHeight, <S::Storage as Storage>::Root) {
        let new_writes = state_accesses.user.ordered_writes.clone();
        // Optimistically compute new root hash
        let new_root =
            compute_state_root::<S>(state_accesses, storage.clone(), rollup_height, slot_number)
                .await;
        if user_roots_match(&self.root, &new_root) {
            tracing::trace!(%rollup_height, %slot_number, "User state root is consistent");
            return (rollup_height, new_root);
        }
        tracing::debug!("User roots don't match, verify if storage became stale");
        // Another possible case is that storage became stale while computing the state root took place.
        if let Some(fetched_root) = fetch_root_hash_if_stale::<S>(&storage, slot_number) {
            if user_roots_match(&self.root, &fetched_root) {
                return (rollup_height, fetched_root);
            }
        }

        tracing::error!(
            %rollup_height,
            %slot_number,
            initial_root = %self.root,
            new_root = %new_root,
            "User state root has changed. This is a bug. Write mismatch fill follow");
        Self::describe_mismatch(&new_writes, &self.writes);

        panic!("User state root has changed at rollup_height={rollup_height} and slot_number={slot_number}. This is a bug.");
    }

    fn describe_mismatch(
        new_write_set: &Vec<(SlotKey, Option<SlotValue>)>,
        old_write_set: &Option<Vec<(SlotKey, Option<SlotValue>)>>,
    ) {
        let Some(old_write_set) = old_write_set else {
            tracing::error!("No old write set to describe mismatch against. This is a bug.");
            return;
        };
        let old_write_set: std::collections::HashMap<SlotKey, Option<SlotValue>> =
            old_write_set.iter().cloned().collect();

        for (new_key, new_value) in new_write_set {
            if let Some(old_value) = old_write_set.get(new_key) {
                if old_value != new_value {
                    tracing::error!(
                        slot_key = %new_key,
                        new_value = %SlotValue::debug_show(new_value.as_ref()),
                        old_value = %SlotValue::debug_show(old_value.as_ref()),
                        "Mismatch between old and new write");
                }
            } else {
                tracing::error!(slot_key = %new_key, "New key not found in old write set");
            }
        }
    }
}

pub(super) struct StateRootBackgroundTaskState<S: Spec> {
    pub request_sender: mpsc::Sender<StateRootComputeRequest<S>>,
}

// If storage has not reached the passed slot number, it will return None.
// If storage is stale on this slot number, it will fetch the existing root hash for the passed slot number.
fn fetch_root_hash_if_stale<S: Spec>(
    storage: &S::Storage,
    slot_number: SlotNumber,
) -> Option<<S::Storage as Storage>::Root> {
    let latest_unbound = storage.latest_version_unbound();
    tracing::trace!(%latest_unbound, %slot_number, "Latest unbound slot number");
    // If the latest version is equal, it means this state root has already been computed.
    if latest_unbound >= slot_number {
        let root = storage
            .get_root_hash_unbound(slot_number)
            .expect("Failed to get root hash");
        Some(root)
    } else {
        None
    }
}

async fn compute_state_root<S: Spec>(
    state_accesses: StateAccesses,
    storage: S::Storage,
    rollup_height: RollupHeight,
    slot_number: SlotNumber,
) -> <S::Storage as Storage>::Root {
    let handle = tokio::runtime::Handle::current().spawn_blocking(move || {
        tracing::span!(tracing::Level::DEBUG, "compute_state_update", scope = "sequencer", %rollup_height, %slot_number)
            .in_scope(|| {
                let prev_root = storage
                    .get_latest_root_hash()
                    .expect("Failed to get prev root hash");

                // First, check if storage is stale from the point of view of the caller.
                if let Some(root) = fetch_root_hash_if_stale::<S>(&storage, slot_number) {
                    return root;
                }

                storage
                    .compute_state_update(state_accesses, &Default::default(), prev_root)
                    .expect("Failed to compute state update").0
            })
    });
    handle.await.unwrap()
}

impl<S: Spec> StateRootBackgroundTaskState<S> {
    pub(super) fn create(
        mut block_excutors_shutdown_receiver: mpsc::Receiver<()>,
        check_state_roots: bool,
    ) -> (tokio::task::JoinHandle<()>, StateRootBackgroundTaskState<S>) {
        tracing::info!("Starting sequencer state root computation background task");
        let (request_sender, mut request_receiver) = mpsc::channel(NUM_STATE_ROOT_COMPUTE_REQUESTS);

        let mut cached_results: BTreeMap<RollupHeight, StateRootCacheEntry<S>> = BTreeMap::new();
        let mut cached_results_size = 0;
        let handle = tokio::spawn(async move {
            loop {
                let request = tokio::select! {
                    maybe_request = request_receiver.recv() => {
                        match maybe_request {
                            Some(request) => request,
                            None => {
                                tracing::info!(
                                    "All state root compute request senders were dropped, shutting down sequencer state root computation loop",
                                );
                                break;
                            }
                        }
                    }
                   _ = block_excutors_shutdown_receiver.recv() => {
                        tracing::info!(
                            "Sequencer state root background task shutdown in response to signal",
                        );
                        break;
                   }
                };
                // Wait for a new request, or shutdown.
                let StateRootComputeRequest::<S> {
                    state_accesses,
                    storage,
                    rollup_height,
                    max_slot_number,
                    response_channel,
                    ..
                } = request;

                // If the entry is in cache, check that the state root is consistent and return early
                if let Some(cached_entry) = cached_results.get(&rollup_height) {
                    tracing::trace!(%rollup_height, "Known state root");
                    // If we're checking that the state roots are equal, we have some work to do.
                    let result = if check_state_roots {
                        cached_entry
                            .assert_consistency(
                                state_accesses,
                                storage,
                                rollup_height,
                                max_slot_number,
                            )
                            .await
                    } else {
                        (rollup_height, cached_entry.root.clone())
                    };
                    let _ = response_channel.send(result);
                    continue;
                }

                // If the entry wasn't in the cache, we'll need to add it. Check if we should save the write set.
                let writes = &state_accesses.user.ordered_writes;
                tracing::trace!(%rollup_height, user_space_writes = writes.len(), "going to compute state root for the new cache entry");
                let (writes_to_save, size) = if check_state_roots {
                    let mut size = 0;
                    for (key, value) in writes {
                        size += key.as_ref().len()
                            + value.as_ref().map(|v| v.value().len()).unwrap_or(0);
                    }
                    // Skip caching the write set if it would take up too much of our available memory. This prevents just a few large write
                    // sets from hogging the cache.
                    if size > MEMORY_LIMIT_FOR_STATE_ROOT_COMPUTATION / 5 {
                        (None, 0)
                    } else {
                        (Some(writes.clone()), size)
                    }
                } else {
                    (None, 0)
                };

                // Compute the new root
                let mut root = compute_state_root::<S>(
                    state_accesses,
                    storage.clone(),
                    rollup_height,
                    max_slot_number,
                )
                .await;
                // Verify, that storage didn't become obsolete while computing state root.
                // If so, use historical state root from the node as canonical one.
                if let Some(fetched_root) = fetch_root_hash_if_stale::<S>(&storage, max_slot_number)
                {
                    if !user_roots_match(&root, &fetched_root) {
                        root = fetched_root;
                    }
                }

                // Add the new entry to the cache
                cached_results.insert(
                    rollup_height,
                    StateRootCacheEntry {
                        root: root.clone(),
                        writes: writes_to_save,
                        size,
                    },
                );
                tracing::trace!(%rollup_height, %root, %size, slot_number = %max_slot_number, "Added new state root to cache");
                cached_results_size += size;

                // Prune the cache if necessary
                Self::prune_cache(&mut cached_results, &mut cached_results_size);
                let _ = response_channel.send((rollup_height, root.clone()));
            }
            tracing::info!(%cached_results_size, "State root background task shutdown");
        });

        (handle, StateRootBackgroundTaskState { request_sender })
    }

    fn prune_cache(
        cached_results: &mut BTreeMap<RollupHeight, StateRootCacheEntry<S>>,
        cached_results_size: &mut usize,
    ) {
        // If the cache is too big, prune the oldest writes sets
        let mut entries = cached_results.values_mut();
        while *cached_results_size > MEMORY_LIMIT_FOR_STATE_ROOT_COMPUTATION {
            let next_entry = entries.next().unwrap(); // Safety: We just checked that the cache is not empty
            *cached_results_size -= next_entry.size;
            next_entry.size = 0;
            next_entry.writes = None;
        }
        // If the cache has too many entries, prune the oldest one.
        if cached_results.len() > MAX_STATE_ROOTS_TO_CACHE {
            cached_results.pop_first().unwrap(); // Safety: We just checked that the cache is not empty
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    use arbitrary::{Arbitrary, Unstructured};
    use rand::rngs::StdRng;
    use rand::{RngCore, SeedableRng};
    use sov_db::storage_manager::NomtStorageManager;
    use sov_state::{OrderedReadsAndWrites, SlotKey};
    use sov_test_utils::storage::{
        CommitingStorageManager, ForklessStorageManager, NonCommitingStorageManager,
        SimpleNomtStorageManager, SimpleStorageManager,
    };
    use sov_test_utils::{MockDaSpec, TestHasher, TestNomtSpec, TestSpec, TestStorageSpec};
    use tokio::task::JoinHandle;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jmt_new_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
        new_rollup_height_state_root_on_stale_storage::<TestSpec, _>(storage_manager).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jmt_known_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
        known_rollup_height_state_root_on_stale_storage::<TestSpec, _>(storage_manager).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_new_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        new_rollup_height_state_root_on_stale_storage::<TestNomtSpec, _>(storage_manager).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_known_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        known_rollup_height_state_root_on_stale_storage::<TestNomtSpec, _>(storage_manager).await;
    }

    // Helpers go below
    fn writes_only_kernel() -> StateAccesses {
        StateAccesses {
            user: Default::default(),
            kernel: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: vec![(
                    SlotKey::from_slice(&b"kernel_key"[..]),
                    Some(SlotValue::from(b"value_1".to_vec())),
                )],
            },
        }
    }

    fn sample_batch() -> StateAccesses {
        StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: vec![(
                    SlotKey::from_slice(&b"user_key"[..]),
                    Some(SlotValue::from(b"value_a".to_vec())),
                )],
            },
            kernel: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: vec![(
                    SlotKey::from_slice(&b"kernel_key"[..]),
                    Some(SlotValue::from(b"value_2".to_vec())),
                )],
            },
        }
    }

    fn start_background_task<S: Spec>() -> (
        StateRootBackgroundTaskState<S>,
        JoinHandle<()>,
        mpsc::Sender<()>,
    ) {
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

        let (handle, task) = StateRootBackgroundTaskState::<S>::create(shutdown_receiver, true);

        (task, handle, shutdown_sender)
    }

    async fn get_root_from_background_task<S: Spec>(
        task: &StateRootBackgroundTaskState<S>,
        storage: S::Storage,
        state_accesses: StateAccesses,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> <S::Storage as Storage>::Root {
        let (response_channel, response_receiver) = oneshot::channel();
        task.request_sender
            .send(StateRootComputeRequest {
                state_accesses,
                storage,
                rollup_height,
                max_slot_number: slot_number,
                response_channel,
            })
            .await
            .unwrap();

        let (received_rollup_height, received_root) =
            tokio::time::timeout(std::time::Duration::from_secs(10), response_receiver)
                .await
                .expect("Timed out waiting for state root computation")
                .expect("State root computation failed");

        assert_eq!(received_rollup_height, rollup_height);
        received_root
    }

    // Start a background task, sender compute request, shutdown, return root computed.
    async fn one_off_check_state_root_computation<S: Spec>(
        storage: S::Storage,
        state_accesses: StateAccesses,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> <S::Storage as Storage>::Root {
        let (task, handle, shutdown_sender) = start_background_task::<S>();

        let received_root = get_root_from_background_task::<S>(
            &task,
            storage,
            state_accesses,
            rollup_height,
            slot_number,
        )
        .await;

        shutdown_sender.try_send(()).unwrap();
        handle.await.unwrap();
        received_root
    }

    fn genesis<S, Sm>(storage_manager: &mut Sm)
    where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
    {
        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = writes_only_kernel();
        let prev_root = <S::Storage as Storage>::PRE_GENESIS_ROOT;
        let (node_new_root, changes) = node_storage
            .compute_state_update(writes_on_the_node, &Default::default(), prev_root)
            .unwrap();
        storage_manager.commit_state_update(node_storage, changes, node_new_root);
    }

    async fn new_rollup_height_state_root_on_stale_storage<S, Sm>(mut storage_manager: Sm)
    where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
        <S::Storage as Storage>::Root: Copy,
    {
        genesis::<S, Sm>(&mut storage_manager);

        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = sample_batch();

        let prev_root = node_storage
            .get_root_hash(node_storage.latest_version())
            .unwrap();
        let (node_new_root, changes) = node_storage
            .compute_state_update(writes_on_the_node, &Default::default(), prev_root)
            .unwrap();

        // Important detail: storage for the background task is created before node changes are committed.
        let storage_for_background = storage_manager.create_prover_storage();
        storage_manager.commit_state_update(node_storage, changes, node_new_root);

        let task_new_root = one_off_check_state_root_computation::<S>(
            storage_for_background,
            sample_batch(),
            RollupHeight::new(1),
            SlotNumber::new(1),
        )
        .await;

        assert_eq!(node_new_root, task_new_root);
    }

    async fn known_rollup_height_state_root_on_stale_storage<S, Sm>(mut storage_manager: Sm)
    where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
        <S::Storage as Storage>::Root: Copy,
    {
        // Genesis
        genesis::<S, Sm>(&mut storage_manager);

        // Starting background task
        let (task, handle, shutdown_sender) = start_background_task::<S>();

        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = sample_batch();
        let rollup_height = RollupHeight::new(1);
        let slot_number = SlotNumber::new(1);
        let prev_root = node_storage
            .get_root_hash(node_storage.latest_version())
            .unwrap();
        let (node_new_root, changes) = node_storage
            .compute_state_update(writes_on_the_node, &Default::default(), prev_root)
            .unwrap();

        let storage_for_background_1 = storage_manager.create_prover_storage();
        let storage_for_background_2 = storage_manager.create_prover_storage();

        // Normal, not stalled
        let received_root_1 = get_root_from_background_task::<S>(
            &task,
            storage_for_background_1,
            sample_batch(),
            rollup_height,
            slot_number,
        )
        .await;

        storage_manager.commit_state_update(node_storage, changes, node_new_root);

        let received_root_2 = get_root_from_background_task::<S>(
            &task,
            storage_for_background_2,
            sample_batch(),
            rollup_height,
            slot_number,
        )
        .await;

        assert_eq!(node_new_root, received_root_1);
        assert_eq!(received_root_1, received_root_2);

        shutdown_sender.try_send(()).unwrap();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jmt_state_compute_competing_storages_repro() {
        let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
        test_compute_competing_storages::<TestSpec, _>(storage_manager, 3).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_state_compute_competing_storages_repro() {
        let mut storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        storage_manager.set_strict_mode(false);
        test_compute_competing_storages::<TestNomtSpec, _>(storage_manager, 3).await;

        let mut storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        storage_manager.set_strict_mode(false);
        test_compute_competing_storages::<TestNomtSpec, _>(storage_manager, 1).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_state_compute_competing_storages_repro_real_storage_manager() {
        sov_test_utils::logging::initialize_or_change_logging_with_filter("error,sov_sequencer::preferred=trace,sov_db::state_db_nomt=trace,sov_state::nomt::prover_storage=debug");
        let storage_manager =
            CommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
        test_compute_competing_storages::<TestNomtSpec, _>(storage_manager, 3).await;
        let storage_manager =
            CommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
        test_compute_competing_storages::<TestNomtSpec, _>(storage_manager, 1).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "Fails because of https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/3030"]
    async fn test_nomt_state_compute_competing_storages_repro_real_storage_manager_non_commiting() {
        let storage_manager =
            NonCommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
        test_compute_competing_storages::<TestNomtSpec, _>(storage_manager, 3).await;
    }

    /// Test is an isolated scenario of sequencer and node interaction, where only 2 pieces are real:
    /// 1. Storage.
    /// 2. Background task for computing state update
    ///
    /// Scenario:
    ///    The test generates a number of batches, where each batch contains state writes in both namespaces.
    ///    The background task then re-executes each batch multiple times:
    ///    first individually, and then as part of larger batches where writes from consecutive batches are merged together.
    ///
    /// This tests the consistency of state root computation when the same logical state changes
    /// are applied in different batch configurations, which can happen when a sequencer runs ahead
    /// of the node's committed state or the node runs ahead.
    async fn test_compute_competing_storages<S, Sm>(mut storage_manager: Sm, seq_ahead_by: usize)
    where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
        <S::Storage as Storage>::Root: Copy,
    {
        assert!(seq_ahead_by > 0, "Test only works with seq_ahead_by > 0");
        let _ahead_by_span =
            tracing::debug_span!("competing_storages", seq_ahead_by = seq_ahead_by).entered();
        genesis::<S, Sm>(&mut storage_manager);
        let preparation_start = std::time::Instant::now();
        let (task, handle, shutdown_sender) = start_background_task::<S>();
        // Double it to fill the channel
        const BLOCKS: usize = NUM_STATE_ROOT_COMPUTE_REQUESTS * 2;

        let mut receivers = Vec::with_capacity(BLOCKS);
        // First, build some "batches", which just writes only StateAccesses.
        // Those batches are going to be consumed by the node
        let batches = generate_test_batches(BLOCKS);
        let mut canonical_root_hashes = Vec::with_capacity(BLOCKS);
        let mut sequencer_cumulative_accesses = Vec::with_capacity(BLOCKS);

        // Then building cumulative batches for sequencer.
        // Each iteration(block) contains `seq_ahead_by` number of cumulative batches.
        // The first cumulative batch has only data from `block - seq_ahead_by`,
        // The second contains data from the first + data from the next, etc.
        for seq_idx in 0..BLOCKS {
            let start = seq_idx.saturating_sub(seq_ahead_by);
            let end = seq_idx;

            let range = start..end;
            let cumulative_accesses = cumulative_accesses_between_batches(&batches[range.clone()]);
            sequencer_cumulative_accesses.push(cumulative_accesses);
        }
        let mut batches: VecDeque<StateAccesses> = batches.into();
        tracing::info!(time = ?preparation_start.elapsed(), "Preparation is completed");

        //
        for (seq_idx, seq_batches) in sequencer_cumulative_accesses.into_iter().enumerate() {
            // Sequencer has received data and wants to compute state roots
            let start = seq_idx.saturating_sub(seq_ahead_by);
            let end = seq_idx;
            let range = start..end;
            let sequencer_storage = storage_manager.create_api_storage();
            let mut sent = 0;
            for (idx, cumulative_batch) in range.zip(seq_batches.into_iter()) {
                let rollup_height = RollupHeight::new(idx as u64 + 1);
                let slot_number = SlotNumber::new(idx as u64 + 1);
                tracing::info!(%rollup_height, "sequencer iteration");

                let (response_channel, response_receiver) = oneshot::channel();
                task.request_sender
                    .send(StateRootComputeRequest {
                        state_accesses: cumulative_batch,
                        storage: sequencer_storage.clone(),
                        rollup_height,
                        max_slot_number: slot_number,
                        response_channel,
                    })
                    .await
                    .unwrap();
                tracing::info!("sent state root compute request");
                sent += 1;
                receivers.push(response_receiver);
            }
            tracing::info!(%seq_idx, %sent, "All state root compute requests this iteration has been sent");
            // Emulates part where node receives something from the DA layer, executes it and commits
            if let Some(idx_for_node) = seq_idx.checked_sub(seq_ahead_by) {
                let start = std::time::Instant::now();
                let rollup_height = idx_for_node + 1;
                let _node_state_compute_span = tracing::debug_span!(
                    "compute_state_update",
                    scope = "node-like",
                    rollup_height,
                )
                .entered();
                let node_batch = batches.pop_front().unwrap();
                let node_storage = storage_manager.create_prover_storage();
                let prev_root = node_storage
                    .get_root_hash(node_storage.latest_version())
                    .unwrap();
                let (node_new_root, changes) = node_storage
                    .compute_state_update(node_batch, &Default::default(), prev_root)
                    .unwrap();
                canonical_root_hashes.push(node_new_root);
                storage_manager.commit_state_update(node_storage, changes, node_new_root);
                tracing::info!(
                    root_hash = %node_new_root,
                    %rollup_height,
                    time = ?start.elapsed(),
                    "commited state update");
            }
        }

        // Validating the results
        let mut results = BTreeMap::<RollupHeight, <S::Storage as Storage>::Root>::new();
        for receiver in receivers {
            let (height, root_hash) = receiver.await.unwrap();
            if let Some(previous) = results.insert(height, root_hash) {
                assert!(
                    user_roots_match(&previous, &root_hash),
                    "Received different user roots for the same height {height}"
                );
            };
        }

        let received_keys = results.keys().cloned().collect::<Vec<_>>();
        // The first block is empty, thus 1 less root hash
        let expected_keys = (1..BLOCKS)
            .map(|i| RollupHeight::new(i as u64))
            .collect::<Vec<_>>();
        assert_eq!(received_keys, expected_keys);

        for ((rollup_height, received_root_hash), canonical_root_hash) in
            results.iter().zip(canonical_root_hashes.iter())
        {
            assert!(
                user_roots_match(received_root_hash, canonical_root_hash),
                "Received different user roots for the same height {rollup_height}"
            );
        }

        shutdown_sender.try_send(()).unwrap();
        handle.await.unwrap();
    }

    fn generate_test_batches(number: usize) -> Vec<StateAccesses> {
        let mut batches = Vec::with_capacity(number);
        let mut rng_seed = [11u8; 32];
        for _ in 0..number {
            let rng = &mut StdRng::from_seed(rng_seed);
            let mut unstructured_seed = [0u8; 500_000];
            rng.fill_bytes(&mut unstructured_seed);
            let mut u = Unstructured::new(&unstructured_seed);

            let batch = StateAccesses::arbitrary(&mut u).unwrap();
            batches.push(batch);

            rng_seed = unstructured_seed[0..32].try_into().unwrap();
        }

        batches
    }

    /// Assembles separate batches into cumulative state accesses.
    /// Let's say batches have accesses A, B, C.
    /// Resulting vector will have, A, A+B, A+B+C.
    /// In the case of identical keys, keys from last accesses are used.
    fn cumulative_accesses_between_batches(batches: &[StateAccesses]) -> Vec<StateAccesses> {
        let mut cumulative_user_writes: HashMap<SlotKey, Option<SlotValue>> = HashMap::new();
        let mut cumulative_kernel_writes: HashMap<SlotKey, Option<SlotValue>> = HashMap::new();

        let mut result = Vec::new();

        for batch in batches {
            let StateAccesses { user, kernel } = batch.clone();
            for (user_key, user_value) in user.ordered_writes {
                cumulative_user_writes.insert(user_key, user_value);
            }
            for (kernel_key, kernel_value) in kernel.ordered_writes {
                cumulative_kernel_writes.insert(kernel_key, kernel_value);
            }

            let cumulative_accesses = state_accesses_from_cumulative_writes(
                &cumulative_user_writes,
                &cumulative_kernel_writes,
            );
            result.push(cumulative_accesses);
        }

        result
    }

    fn state_accesses_from_cumulative_writes(
        cumulative_user_writes: &HashMap<SlotKey, Option<SlotValue>>,
        cumulative_kernel_writes: &HashMap<SlotKey, Option<SlotValue>>,
    ) -> StateAccesses {
        let mut cumulative_accesses = StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: cumulative_user_writes
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            },
            kernel: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: cumulative_kernel_writes
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            },
        };

        // Sort them for better readability.
        cumulative_accesses
            .user
            .ordered_writes
            .sort_by_key(|(k, _v)| k.clone());
        cumulative_accesses
            .kernel
            .ordered_writes
            .sort_by_key(|(k, _v)| k.clone());

        cumulative_accesses
    }
}
