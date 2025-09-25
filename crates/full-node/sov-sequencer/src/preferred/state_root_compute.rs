//! Defines the logic for computing state roots for the preferred sequencer.

use std::collections::BTreeMap;
use std::sync::Arc;

use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{CryptoSpec, Runtime};
// use sov_modules_api::capabilities::KernelWithSlotMapping;
use sov_modules_api::{Spec, Storage};
use sov_rollup_interface::common::SlotNumber;
use sov_state::sequencer_state::{RawStateChanges, SequencerStateChanges};
use sov_state::{NativeStorage, SlotKey, SlotValue, StateAccesses, StateRoot};
use tokio::sync::{mpsc, oneshot};

/// The memory limit for old write sets that we keep around. These old sets are useful because they tell us what keys have been changed.
/// Which lets us output a much handier error message when the state root computation changes.
const MEMORY_LIMIT_FOR_STATE_ROOT_COMPUTATION: usize = 100_000_000; // 100MB
const MAX_STATE_ROOTS_TO_CACHE: usize = 100;
const NUM_STATE_ROOT_COMPUTE_REQUESTS: usize = 50;

type Hasher<S> = <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher;
pub(crate) struct StateRootComputeRequest<S: Spec> {
    pub raw_state_changes: Arc<RawStateChanges>,
    pub uncommitted_changes: SequencerStateChanges<Hasher<S>>,
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
    async fn assert_consistency<Rt: Runtime<S>>(
        &self,
        state_accesses: StateAccesses,
        storage: S::Storage,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> (RollupHeight, <S::Storage as Storage>::Root) {
        let new_writes = state_accesses.user.ordered_writes.clone();
        // Optimistically compute new root hash
        let new_root = compute_state_root::<S, Rt>(
            state_accesses,
            storage.clone(),
            rollup_height,
            slot_number,
        )
        .await;
        if user_roots_match(&self.root, &new_root) {
            tracing::trace!(%rollup_height, %slot_number, "User state root is consistent");
            return (rollup_height, new_root);
        }
        tracing::debug!("User roots don't match, verify if storage became stale");
        // Another possible case is that storage became stale while computing the state root took place.
        if let Some(fetched_root) = fetch_root_hash_if_stale::<S, Rt>(&storage, rollup_height) {
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
fn fetch_root_hash_if_stale<S: Spec, Rt: Runtime<S>>(
    storage: &S::Storage,
    rollup_height: RollupHeight,
) -> Option<<S::Storage as Storage>::Root> {
    let runtime = Rt::default();
    let kernel = runtime.kernel_with_slot_mapping();
    let latest_rollup_height_in_storage = kernel.get_latest_rollup_height(storage);

    tracing::trace!(%latest_rollup_height_in_storage, %rollup_height, "Latest unbound rollup height");
    // If the latest version is equal, it means this state root has already been computed.
    if latest_rollup_height_in_storage >= rollup_height {
        let runtime = Rt::default();
        let kernel_with_slot_mapping = runtime.kernel_with_slot_mapping();
        // There are two cases here:
        // If the underlying storage is ahead of the requested state by *more than one* rollup block, then the `kernel` will have its `true_slot_number_history` populated for the requested height.
        // In this case, we can retrieve the slot number which goes with the requested height and fetch its state root
        // Otherwise, the map will be empty. If the map is empty, we *know* that we're in this case - which means we can just return the latest root hash.
        if let Some(slot_number_for_height) = kernel_with_slot_mapping
            .get_true_slot_number_for_height_unbound(rollup_height, &storage)
        {
            tracing::info!(rollup_height = %rollup_height, "Found historical slot number for height. Fetching root hash for slot number {}", slot_number_for_height );
            let root = storage
                .get_root_hash_unbound(slot_number_for_height)
                .expect("Failed to get root hash");
            Some(root)
        } else {
            tracing::info!(rollup_height = %rollup_height, "No historical slot number for height. Fetching root hash for latest slot");
            Some(
                storage
                    .get_latest_root_hash_unbound()
                    .expect("Failed to get latest root hash"),
            )
        }
    } else {
        tracing::info!(rollup_height = %rollup_height, latest_height_in_storage = %latest_rollup_height_in_storage, "Storage is behind requested height. Returning None");
        None
    }
}

async fn compute_state_root<S: Spec, Rt: Runtime<S>>(
    state_accesses: StateAccesses,
    storage: S::Storage,
    rollup_height: RollupHeight,
    slot_number: SlotNumber,
) -> <S::Storage as Storage>::Root {
    tracing::warn!(
        num_user_writes = state_accesses.user.ordered_writes.len(),
        num_kernel_writes = state_accesses.kernel.ordered_writes.len(),
        "Computing state root for height {}",
        rollup_height
    );
    for (key, value) in state_accesses.user.ordered_writes.iter() {
        tracing::info!(key = %key, value = ?value, "User write");
    }
    let handle = tokio::runtime::Handle::current().spawn_blocking(move || {
        tracing::span!(tracing::Level::DEBUG, "compute_state_update", scope = "sequencer", %rollup_height, %slot_number)
            .in_scope(|| {
                let prev_root = storage
                    .get_latest_root_hash()
                    .expect("Failed to get prev root hash");

                // First, check if storage is stale from the point of view of the caller.
                if let Some(root) = fetch_root_hash_if_stale::<S, Rt>(&storage, rollup_height) {
                    return root;
                }

                storage
                    .compute_state_update(state_accesses, &Default::default(), prev_root)
                    .expect("Failed to compute state update").0
            })
    });
    let res = handle.await.unwrap();
    tracing::warn!(root = %res, "Computed state root for height {}", rollup_height);
    res
}

impl<S: Spec> StateRootBackgroundTaskState<S> {
    pub(super) fn create<Rt: Runtime<S>>(
        mut block_excutors_shutdown_receiver: mpsc::Receiver<()>,
        check_state_roots: bool,
    ) -> (tokio::task::JoinHandle<()>, StateRootBackgroundTaskState<S>) {
        let span = tracing::span!(tracing::Level::DEBUG, "state_root_compute_background_task");
        let _enter = span.enter();
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
                    raw_state_changes,
                    mut uncommitted_changes,
                    storage,
                    rollup_height,
                    max_slot_number,
                    response_channel,
                    ..
                } = request;
                uncommitted_changes.push_front(raw_state_changes);
                let state_accesses = uncommitted_changes.to_state_accesses();
                // If the entry is in cache, check that the state root is consistent and return early
                if let Some(cached_entry) = cached_results.get(&rollup_height) {
                    tracing::trace!(%rollup_height, "Known state root");
                    // If we're checking that the state roots are equal, we have some work to do.
                    let result = if check_state_roots {
                        cached_entry
                            .assert_consistency::<Rt>(
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
                let mut root = compute_state_root::<S, Rt>(
                    state_accesses,
                    storage.clone(),
                    rollup_height,
                    max_slot_number,
                )
                .await;
                // Verify, that storage didn't become obsolete while computing state root.
                // If so, use historical state root from the node as canonical one.
                if let Some(fetched_root) =
                    fetch_root_hash_if_stale::<S, Rt>(&storage, rollup_height)
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
    use sov_modules_api::capabilities::ChainState;
    use sov_modules_api::{KernelStateAccessor, VisibleSlotNumber};
    use sov_state::SlotKey;
    use sov_modules_api::VersionReader;
    use sov_test_utils::storage::{
        ForklessStorageManager, 
        SimpleNomtStorageManager, SimpleStorageManager,
    };
    use sov_test_utils::{
        generate_optimistic_runtime, TestNomtSpec, TestSpec,
        TestStorageSpec,
    };
    use tokio::task::JoinHandle;
    use sov_modules_api::StateCheckpoint;

    generate_optimistic_runtime!(TestRuntime <=);

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jmt_new_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
        new_rollup_height_state_root_on_stale_storage::<TestSpec, _, TestRuntime<TestSpec>>(
            storage_manager,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jmt_known_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
        known_rollup_height_state_root_on_stale_storage::<TestSpec, _, TestRuntime<TestSpec>>(
            storage_manager,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_new_rollup_height_state_root_on_stale_storage() {
        let storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        new_rollup_height_state_root_on_stale_storage::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(storage_manager).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nomt_known_rollup_height_state_root_on_stale_storage() {
        sov_test_utils::initialize_logging();
        let storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
        known_rollup_height_state_root_on_stale_storage::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(storage_manager).await;
    }

    // Helpers go below
    fn writes_only_kernel<S: Spec, Rt: Runtime<S>>(storage: &S::Storage) -> StateAccesses {
        let mut rt = Rt::default();
        let mut kernel = rt.kernel();
        let mut checkpoint = StateCheckpoint::new(storage.clone(), &kernel);
        // let mut state_with_partially_stale_heights = KernelStateAccessor::from_checkpoint(&kernel, &mut checkpoint);
        // Increment the rollup height so that our storage height tracking works correctly. This is required because we now rely on rollup heights rather than slot numbers.
        // kernel.increment_rollup_height(&mut state_with_partially_stale_heights, VisibleSlotNumber::new_dangerous(0));
        // checkpoint.commit_revertable_storage_cache();
        tracing::info!(rollup_height = %checkpoint.rollup_height_to_access(), "Kernel only writes incremented rollup height");

        let (mut state_accesses, _, _) = checkpoint.freeze();
        state_accesses.kernel.ordered_writes.push((
            SlotKey::from_slice(&b"kernel_key"[..]),
            Some(SlotValue::from(b"value_1".to_vec())),
        ));
        state_accesses
    }

    fn sample_batch<S: Spec, Rt: Runtime<S>>(storage: &S::Storage) -> Arc<RawStateChanges> {
        let mut rt = Rt::default();
        let mut kernel = rt.kernel();
        let mut checkpoint = StateCheckpoint::new(storage.clone(), &kernel);
        let mut state_with_partially_stale_heights =
            KernelStateAccessor::from_checkpoint(&kernel, &mut checkpoint);
        // Increment the rollup height so that our storage height tracking works correctly. This is required because we now rely on rollup heights rather than slot numbers.
        kernel.increment_rollup_height(
            &mut state_with_partially_stale_heights,
            VisibleSlotNumber::new_dangerous(1),
        );
        tracing::info!(rollup_height = %checkpoint.rollup_height_to_access(), "Sample batch incremented rollup height");
        checkpoint.commit_revertable_storage_cache();
        let mut changes = checkpoint.to_raw_state_changes();
        changes.user.set(
            &SlotKey::from_slice(&b"user_key"[..]),
            SlotValue::from(b"value_a".to_vec()),
        );
        changes.kernel.set(
            &SlotKey::from_slice(&b"kernel_key"[..]),
            SlotValue::from(b"value_2".to_vec()),
        );
        changes.user.commit_revertable_storage_cache();
        changes.kernel.commit_revertable_storage_cache();

        Arc::new(changes)
    }

    fn start_background_task<S: Spec, Rt: Runtime<S>>() -> (
        StateRootBackgroundTaskState<S>,
        JoinHandle<()>,
        mpsc::Sender<()>,
    ) {
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

        let (handle, task) =
            StateRootBackgroundTaskState::<S>::create::<Rt>(shutdown_receiver, true);

        (task, handle, shutdown_sender)
    }

    async fn get_root_from_background_task<S: Spec>(
        task: &StateRootBackgroundTaskState<S>,
        storage: S::Storage,
        raw_state_changes: Arc<RawStateChanges>,
        uncommitted_changes: SequencerStateChanges<Hasher<S>>,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> <S::Storage as Storage>::Root {
        let (response_channel, response_receiver) = oneshot::channel();
        task.request_sender
            .send(StateRootComputeRequest {
                raw_state_changes,
                uncommitted_changes,
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
    async fn one_off_check_state_root_computation<S: Spec, Rt: Runtime<S>>(
        storage: S::Storage,
        state_accesses: Arc<RawStateChanges>,
        rollup_height: RollupHeight,
        slot_number: SlotNumber,
    ) -> <S::Storage as Storage>::Root {
        let (task, handle, shutdown_sender) = start_background_task::<S, Rt>();

        let received_root = get_root_from_background_task::<S>(
            &task,
            storage,
            state_accesses,
            SequencerStateChanges::default(),
            rollup_height,
            slot_number,
        )
        .await;

        shutdown_sender.try_send(()).unwrap();
        handle.await.unwrap();
        received_root
    }

    fn genesis<S, Sm, Rt: Runtime<S>>(storage_manager: &mut Sm)
    where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
    {
        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = writes_only_kernel::<S, Rt>(&node_storage);
        let prev_root = <S::Storage as Storage>::PRE_GENESIS_ROOT;
        let (node_new_root, changes) = node_storage
            .compute_state_update(writes_on_the_node, &Default::default(), prev_root)
            .unwrap();
        storage_manager.commit_state_update(node_storage, changes, node_new_root);
    }

    async fn new_rollup_height_state_root_on_stale_storage<S, Sm, Rt: Runtime<S>>(
        mut storage_manager: Sm,
    ) where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
        <S::Storage as Storage>::Root: Copy,
    {
        genesis::<S, Sm, Rt>(&mut storage_manager);

        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = sample_batch::<S, Rt>(&node_storage);

        let prev_root = node_storage
            .get_root_hash(node_storage.latest_version())
            .unwrap();
        let (node_new_root, changes) = node_storage
            .compute_state_update(
                writes_on_the_node.to_state_accesses_for_sequencer_state_root_computation(),
                &Default::default(),
                prev_root,
            )
            .unwrap();

        // Important detail: storage for the background task is created before node changes are committed.
        let storage_for_background = storage_manager.create_prover_storage();
        storage_manager.commit_state_update(node_storage, changes, node_new_root);

        let task_new_root = one_off_check_state_root_computation::<S, Rt>(
            storage_for_background,
            writes_on_the_node,
            RollupHeight::new(1),
            SlotNumber::new(1),
        )
        .await;

        assert_eq!(node_new_root, task_new_root);
    }

    async fn known_rollup_height_state_root_on_stale_storage<S, Sm, Rt: Runtime<S>>(
        mut storage_manager: Sm,
    ) where
        S: Spec,
        Sm: ForklessStorageManager<Storage = S::Storage>,
        S::Storage: NativeStorage,
        <S::Storage as Storage>::Root: Copy,
    {
        // Genesis
        genesis::<S, Sm, Rt>(&mut storage_manager);

        // Starting background task
        let (task, handle, shutdown_sender) = start_background_task::<S, Rt>();

        let node_storage = storage_manager.create_prover_storage();
        let writes_on_the_node = sample_batch::<S, Rt>(&node_storage);
        let rollup_height = RollupHeight::new(1);
        let slot_number = SlotNumber::new(1);
        let prev_root = node_storage
            .get_root_hash(node_storage.latest_version())
            .unwrap();
        tracing::info!(%rollup_height, %slot_number, "Computing node state root");
        let (node_new_root, changes) = node_storage
            .compute_state_update(
                writes_on_the_node.to_state_accesses_for_sequencer_state_root_computation(),
                &Default::default(),
                prev_root,
            )
            .unwrap();

        let storage_for_background_1 = storage_manager.create_prover_storage();
        let storage_for_background_2 = storage_manager.create_prover_storage();

        tracing::info!(%rollup_height, %slot_number, "Computing state root from background task 1");
        // Normal, not stalled
        let received_root_1 = get_root_from_background_task::<S>(
            &task,
            storage_for_background_1,
            writes_on_the_node.clone(),
            Default::default(),
            rollup_height,
            slot_number,
        )
        .await;

        tracing::info!(%rollup_height, %slot_number, "Committing state update");
        storage_manager.commit_state_update(node_storage, changes, node_new_root);
        tracing::info!(%rollup_height, %slot_number, "Committed state update");

        tracing::info!(%rollup_height, %slot_number, "Computing state root from background task 2");
        let received_root_2: <<S as Spec>::Storage as Storage>::Root =
            get_root_from_background_task::<S>(
                &task,
                storage_for_background_2,
                writes_on_the_node.clone(),
                Default::default(),
                rollup_height,
                slot_number,
            )
            .await;

        assert_eq!(node_new_root, received_root_1);
        assert_eq!(received_root_1, received_root_2);

        shutdown_sender.try_send(()).unwrap();
        handle.await.unwrap();
    }

    // #[tokio::test(flavor = "multi_thread")]
    // async fn test_jmt_state_compute_competing_storages_repro() {
    //     let storage_manager = SimpleStorageManager::<TestStorageSpec>::new();
    //     test_compute_competing_storages::<TestSpec, _, TestRuntime<TestSpec>>(storage_manager, 3)
    //         .await;
    // }

    // #[tokio::test(flavor = "multi_thread")]
    // async fn test_nomt_state_compute_competing_storages_repro() {
    //     let mut storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
    //     storage_manager.set_strict_mode(false);
    //     test_compute_competing_storages::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(
    //         storage_manager,
    //         3,
    //     )
    //     .await;

    //     let mut storage_manager = SimpleNomtStorageManager::<TestStorageSpec>::new();
    //     storage_manager.set_strict_mode(false);
    //     test_compute_competing_storages::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(
    //         storage_manager,
    //         1,
    //     )
    //     .await;
    // }

    // #[tokio::test(flavor = "multi_thread")]
    // async fn test_nomt_state_compute_competing_storages_repro_real_storage_manager() {
    //     sov_test_utils::logging::initialize_or_change_logging_with_filter("error,sov_sequencer::preferred=trace,sov_db::state_db_nomt=trace,sov_state::nomt::prover_storage=debug");
    //     let storage_manager =
    //         CommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
    //     test_compute_competing_storages::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(
    //         storage_manager,
    //         3,
    //     )
    //     .await;
    //     let storage_manager =
    //         CommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
    //     test_compute_competing_storages::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(
    //         storage_manager,
    //         1,
    //     )
    //     .await;
    // }

    // #[tokio::test(flavor = "multi_thread")]
    // #[ignore = "Fails because of https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/3030"]
    // async fn test_nomt_state_compute_competing_storages_repro_real_storage_manager_non_commiting() {
    //     let storage_manager =
    //         NonCommitingStorageManager::<NomtStorageManager<MockDaSpec, TestHasher, _>, _>::new();
    //     test_compute_competing_storages::<TestNomtSpec, _, TestRuntime<TestNomtSpec>>(
    //         storage_manager,
    //         3,
    //     )
    //     .await;
    // }
    // /// Test is an isolated scenario of sequencer and node interaction, where only 2 pieces are real:
    // /// 1. Storage.
    // /// 2. Background task for computing state update
    // ///
    // /// Scenario:
    // ///    The test generates a number of batches, where each batch contains state writes in both namespaces.
    // ///    The background task then re-executes each batch multiple times:
    // ///    first individually, and then as part of larger batches where writes from consecutive batches are merged together.
    // ///
    // /// This tests the consistency of state root computation when the same logical state changes
    // /// are applied in different batch configurations, which can happen when a sequencer runs ahead
    // /// of the node's committed state or the node runs ahead.
    // async fn test_compute_competing_storages<S, Sm, Rt: Runtime<S>>(
    //     mut storage_manager: Sm,
    //     seq_ahead_by: usize,
    // ) where
    //     S: Spec,
    //     Sm: ForklessStorageManager<Storage = S::Storage>,
    //     S::Storage: NativeStorage,
    //     <S::Storage as Storage>::Root: Copy,
    // {
    //     assert!(seq_ahead_by > 0, "Test only works with seq_ahead_by > 0");
    //     let _ahead_by_span =
    //         tracing::debug_span!("competing_storages", seq_ahead_by = seq_ahead_by).entered();
    //     genesis::<S, Sm, Rt>(&mut storage_manager);
    //     let preparation_start = std::time::Instant::now();
    //     let (task, handle, shutdown_sender) = start_background_task::<S, Rt>();
    //     // Double it to fill the channel
    //     const BLOCKS: usize = NUM_STATE_ROOT_COMPUTE_REQUESTS * 2;
    //     let mut receivers = Vec::with_capacity(BLOCKS);
    //     // First, build some "batches", which just writes only StateAccesses.
    //     // Those batches are going to be consumed by the node
    //     let batches = generate_test_batches(BLOCKS);
    //     let mut canonical_root_hashes = Vec::with_capacity(BLOCKS);
    //     let mut uncomitted_changes_by_block = Vec::with_capacity(BLOCKS);

    //     // Then building cumulative batches for sequencer.
    //     // Each iteration(block) contains `seq_ahead_by` number of cumulative batches.
    //     // The first cumulative batch has only data from `block - seq_ahead_by`,
    //     // The second contains data from the first + data from the next, etc.
    //     for seq_idx in 0..BLOCKS {
    //         let start = seq_idx.saturating_sub(seq_ahead_by);
    //         let end = seq_idx;

    //         let range = start..end;
    //         let uncommitted_changes = batches[range].to_vec();
    //         uncomitted_changes_by_block.push(uncommitted_changes);
    //     }
    //     let batches: VecDeque<_> = batches.into();
    //     tracing::info!(time = ?preparation_start.elapsed(), "Preparation is completed");

    //     for (seq_idx, (new_batch, uncommitted_changes)) in
    //         (batches.into_iter().zip(uncomitted_changes_by_block)).enumerate()
    //     {
    //         // Sequencer has received data and wants to compute state roots
    //         let sequencer_storage = storage_manager.create_api_storage();
    //         let mut sent = 0;
    //         let (response_channel, response_receiver) = oneshot::channel();
    //         // Emulate the sequencer receiving data and computing state roots
    //         task.request_sender
    //             .send(StateRootComputeRequest {
    //                 raw_state_changes: new_batch.clone(),
    //                 uncommitted_changes: SequencerStateChanges {
    //                     changes: Some(uncommitted_changes.into()),
    //                     phantom: std::marker::PhantomData,
    //                 },
    //                 storage: sequencer_storage.clone(),
    //                 rollup_height: RollupHeight::new((seq_idx as u64) + 1),
    //                 max_slot_number: SlotNumber::new((seq_idx as u64) + 1),
    //                 response_channel,
    //             })
    //             .await
    //             .unwrap();
    //         tracing::info!("sent state root compute request");
    //         sent += 1;
    //         receivers.push(response_receiver);

    //         tracing::info!(%seq_idx, %sent, "All state root compute requests this iteration has been sent");
    //         // Emulates part where node receives something from the DA layer, executes it and commits
    //         if let Some(idx_for_node) = seq_idx.checked_sub(seq_ahead_by) {
    //             let start = std::time::Instant::now();
    //             let rollup_height = idx_for_node + 1;
    //             let _node_state_compute_span = tracing::debug_span!(
    //                 "compute_state_update",
    //                 scope = "node-like",
    //                 rollup_height,
    //             )
    //             .entered();
    //             let node_batch = new_batch;
    //             let node_storage = storage_manager.create_prover_storage();
    //             let prev_root = node_storage
    //                 .get_root_hash(node_storage.latest_version())
    //                 .unwrap();
    //             let (node_new_root, changes) = node_storage
    //                 .compute_state_update(
    //                     node_batch.to_state_accesses_for_sequencer_state_root_computation(),
    //                     &Default::default(),
    //                     prev_root,
    //                 )
    //                 .unwrap();
    //             canonical_root_hashes.push(node_new_root);
    //             storage_manager.commit_state_update(node_storage, changes, node_new_root);
    //             tracing::info!(
    //                 root_hash = %node_new_root,
    //                 %rollup_height,
    //                 time = ?start.elapsed(),
    //                 "commited state update");
    //         }
    //     }

    //     // Validating the results
    //     let mut results = BTreeMap::<RollupHeight, <S::Storage as Storage>::Root>::new();
    //     for receiver in receivers {
    //         let (height, root_hash) = receiver.await.unwrap();
    //         if let Some(previous) = results.insert(height, root_hash) {
    //             assert!(
    //                 user_roots_match(&previous, &root_hash),
    //                 "Received different user roots for the same height {height}"
    //             );
    //         };
    //     }

    //     let received_keys = results.keys().cloned().collect::<Vec<_>>();
    //     // The first block is empty, thus 1 less root hash
    //     let expected_keys = (1..BLOCKS)
    //         .map(|i| RollupHeight::new(i as u64))
    //         .collect::<Vec<_>>();
    //     assert_eq!(received_keys, expected_keys);

    //     for ((rollup_height, received_root_hash), canonical_root_hash) in
    //         results.iter().zip(canonical_root_hashes.iter())
    //     {
    //         assert!(
    //             user_roots_match(received_root_hash, canonical_root_hash),
    //             "Received different user roots for the same height {rollup_height}"
    //         );
    //     }

    //     shutdown_sender.try_send(()).unwrap();
    //     handle.await.unwrap();
    // }

    // fn generate_test_batches(number: usize) -> Vec<Arc<RawStateChanges>> {
    //     let mut batches = Vec::with_capacity(number);
    //     let mut rng_seed = [11u8; 32];
    //     for _ in 0..number {
    //         let rng = &mut StdRng::from_seed(rng_seed);
    //         let mut unstructured_seed = [0u8; 500_000];
    //         rng.fill_bytes(&mut unstructured_seed);
    //         let mut u = Unstructured::new(&unstructured_seed);

    //         let mut changes = RawStateChanges::default();
    //         let mut user_writes = u.arbitrary::<Vec<(SlotKey, Option<SlotValue>)>>().unwrap();
    //         let mut kernel_writes = u.arbitrary::<Vec<(SlotKey, Option<SlotValue>)>>().unwrap();
    //         // Ensure that the user and kernel writes are both non-empty
    //         user_writes.push(u.arbitrary().unwrap());
    //         kernel_writes.push(u.arbitrary().unwrap());
    //         for (key, value) in user_writes {
    //             if let Some(value) = value {
    //                 changes.user.set(&key, value);
    //             } else {
    //                 changes.user.delete(&key);
    //             }
    //         }
    //         for (key, value) in kernel_writes {
    //             if let Some(value) = value {
    //                 changes.kernel.set(&key, value);
    //             } else {
    //                 changes.kernel.delete(&key);
    //             }
    //         }
    //         changes.user.commit_revertable_storage_cache();
    //         changes.kernel.commit_revertable_storage_cache();

    //         batches.push(Arc::new(changes));
    //         rng_seed = unstructured_seed[0..32].try_into().unwrap();
    //     }

    //     batches
    // }
}
