use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
#[cfg(feature = "native")]
use sov_state::StateGetter;
use sov_state::{IsValueCached, Namespace, SlotKey, SlotValue, StateAccesses, Storage};
use tracing::trace;

use super::internals::{AccessoryDelta, Delta};
use super::temp_cache::{CacheLookup, TempCache};
use super::{BootstrapWorkingSet, BorshSerializedSize, UniversalStateAccessor};
use crate::capabilities::{Kernel, RollupHeight};
use crate::state::accessors::internals::FirstTimeReads;
use crate::state::traits::PerBlockCache;
#[cfg(feature = "native")]
use crate::TxChangeSet;
use crate::{GasMeter, Spec, VersionReader};
#[cfg(feature = "native")]
use sov_state::sequencer_state::RawStateChanges;

/// This structure is responsible for storing the `read-write` set.
///
/// A [`StateCheckpoint`] can be obtained from a [`crate::WorkingSet`] in two ways:
///  1. With [`crate::TxScratchpad::commit`].
///  2. With [`crate::WorkingSet::revert`].
#[derive(derive_more::Debug)]
pub struct StateCheckpoint<S: Spec> {
    #[debug(skip)]
    pub(super) delta: Delta<S::Storage>,
    /// The rollup height visible to user-space modules
    pub(super) visible_slot_num: VisibleSlotNumber,
    pub(super) rollup_height: RollupHeight,
    pub(super) cache: TempCache,
    pub(super) metrics: StateMetrics,
}

#[cfg(feature = "native")]
impl<S: Spec> StateCheckpoint<S> {
    /// Convert the state checkpoint to a [`RawStateChanges`] instance.
    pub fn to_raw_state_changes(mut self) -> RawStateChanges {
        self.delta.commit_revertable_storage_cache();
        RawStateChanges {
            user: self.delta.user_cache.into(),
            kernel: self.delta.kernel_cache.into(),
            accessory: self.delta.accessory_writes,
            rollup_height: self.rollup_height.get(),
        }
    }
}

#[derive(Debug, Clone)]
/// The list of changes from the state checkpoint
pub struct ChangeSet {
    #[allow(missing_docs)]
    pub changes: Vec<((SlotKey, sov_state::Namespace), Option<SlotValue>)>,
}

impl ChangeSet {
    /// Create a new `ChangeSet` from a vector of changes.
    #[must_use]
    pub fn new(changes: Vec<((SlotKey, sov_state::Namespace), Option<SlotValue>)>) -> Self {
        Self { changes }
    }
}

impl<S: Spec> StateCheckpoint<S> {
    /// Check if key is in the cache.
    pub fn is_value_cached(&self, namespace: Namespace, key: &SlotKey) -> IsValueCached {
        self.delta.is_value_cached(namespace, key)
    }

    /// Keys and values that were read for the first time.
    pub fn first_reads(&self) -> FirstTimeReads {
        self.delta.first_reads()
    }

    /// Commits the revertable part of the `StateCheckpoint` cache.
    pub fn commit_revertable_storage_cache(&mut self) {
        self.delta.commit_revertable_storage_cache();
    }

    /// Discards the revertable part of the `StateCheckpoint` cache.
    pub fn discard_revertable_storage_cache(&mut self) {
        self.delta.discard_revertable_storage_cache();
    }

    /// Deep copy the state checkpoint (including its state caches), ignoring
    /// the witness and the temp cache.
    ///
    /// Since this method leaves the witness of the new
    /// checkpoint in a state that is inconsistent with its caches,
    /// it should only be used in situations where the witness is not needed,
    /// such as in the API accessors.
    #[must_use]
    #[cfg(feature = "native")]
    pub fn clone_with_empty_witness_dropping_temp_cache(&self) -> Self {
        Self {
            delta: self.delta.clone_with_empty_witness(),
            visible_slot_num: self.visible_slot_num,
            rollup_height: self.rollup_height,
            cache: TempCache::new(),
            metrics: StateMetrics::default(),
        }
    }

    /// Creates a new [`StateCheckpoint`] instance with the given intermediate state that will be checked before storage when a value isn't already present in the checkpoint.
    #[cfg(feature = "native")]
    pub fn new_with_uncomitted_changes<K: Kernel<S>>(
        inner: S::Storage,
        kernel: &K,
        uncomitted_changes: Box<dyn StateGetter>,
    ) -> Self {
        Self::with_witness_and_uncomitted_changes(
            inner,
            Default::default(),
            kernel,
            Some(uncomitted_changes),
        )
    }

    /// Replace the storage and intermediate state underlying the checkpoint in place. It is up to the caller
    /// to ensure that the intermediate state is compatible with the new storage.
    #[cfg(feature = "native")]
    pub fn replace_storage(&mut self, inner: S::Storage, uncomitted_changes: Box<dyn StateGetter>) {
        self.delta.inner = inner;
        self.delta.uncomitted_changes = Some(uncomitted_changes);
    }

    /// Returns a reference to the storage underlying the state checkpoint.
    #[cfg(feature = "native")]
    pub fn storage(&self) -> &S::Storage {
        self.delta.inner()
    }

    /// Returns the rollup height of the latest data saved in the underlying storage.
    #[cfg(feature = "native")]
    pub fn get_rollup_height_of_underlying_storage<K: Kernel<S>>(
        &self,
        kernel: &K,
    ) -> RollupHeight {
        let new_checkpoint = Self::new(self.delta.inner().clone(), kernel);
        new_checkpoint.rollup_height
    }

    /// Creates a new [`StateCheckpoint`] instance without any changes, backed
    /// by the given [`Storage`].
    pub fn new<K: Kernel<S>>(inner: S::Storage, kernel: &K) -> Self {
        Self::with_witness(inner, Default::default(), kernel)
    }

    /// Creates a new [`StateCheckpoint`] instance without any changes, backed
    /// by the given [`Storage`] and witness.
    pub fn with_witness<K: Kernel<S>>(
        inner: S::Storage,
        witness: <S::Storage as Storage>::Witness,
        kernel: &K,
    ) -> Self {
        Self::with_witness_and_uncomitted_changes(
            inner,
            witness,
            kernel,
            #[cfg(feature = "native")]
            None,
        )
    }

    /// Creates a new [`StateCheckpoint`] instance without any changes, backed
    /// by the given [`Storage`] and witness.
    fn with_witness_and_uncomitted_changes<K: Kernel<S>>(
        inner: S::Storage,
        witness: <S::Storage as Storage>::Witness,
        kernel: &K,
        #[cfg(feature = "native")] uncomitted_changes: Option<Box<dyn StateGetter>>,
    ) -> Self {
        let mut delta = Delta::with_witness(inner, witness);
        #[cfg(feature = "native")]
        {
            delta.uncomitted_changes = uncomitted_changes;
        }
        let mut metrics = StateMetrics::default();
        let mut bootstrap_state = BootstrapWorkingSet {
            inner: &mut delta,
            metrics: &mut metrics,
        };

        let visible_slot_num = kernel.next_visible_slot_number(&mut bootstrap_state);
        trace!(%visible_slot_num, "Initializing a `StateCheckpoint`");
        let rollup_height = kernel.rollup_height(&mut bootstrap_state);
        Self {
            delta,
            visible_slot_num,
            rollup_height,
            cache: TempCache::new(),
            metrics: StateMetrics::default(),
        }
    }

    /// Extracts ordered reads, writes, and witness from this [`StateCheckpoint`].
    ///
    /// Note that this data is moved **out** of the [`StateCheckpoint`] i.e. it can't be extracted twice.
    pub fn freeze(
        self,
    ) -> (
        StateAccesses,
        AccessoryDelta<S::Storage>,
        <S::Storage as Storage>::Witness,
    ) {
        let (state_accesses, accesory_delta, witness, _storage) = self.delta.freeze();
        (state_accesses, accesory_delta, witness)
    }

    #[cfg(feature = "native")]
    /// Extracts the accessory delta from this [`StateCheckpoint`].
    pub fn take_accessory_delta(&mut self) -> AccessoryDelta<S::Storage> {
        self.delta.take_accessory_delta()
    }

    #[cfg(feature = "native")]
    /// Extracts the accessory delta from this [`StateCheckpoint`].
    pub fn set_accessory_delta(&mut self, accessory_delta: AccessoryDelta<S::Storage>) {
        self.delta.set_accessory_delta(accessory_delta);
    }

    /// Extracts ordered reads, writes, and witness from this [`StateCheckpoint`] and uses
    /// them to compute the `StateUpdate` created by this `StateCheckpoint`.
    #[allow(clippy::type_complexity)]
    pub fn materialize_update(
        self,
        prev_state_root: <S::Storage as Storage>::Root,
    ) -> (
        <S::Storage as Storage>::Root,
        <S::Storage as Storage>::StateUpdate,
        AccessoryDelta<S::Storage>,
        <S::Storage as Storage>::Witness,
        S::Storage,
    ) {
        let (cache_log, accessory_delta, witness, storage) = self.delta.freeze();
        let _span = tracing::debug_span!("compute_state_root", scope = "node").entered();
        let (root, update) = storage
            .compute_state_update(cache_log, &witness, prev_state_root)
            .expect("state update computation must succeed");
        tracing::trace!(%root, "computed state root");
        (root, update, accessory_delta, witness, storage)
    }

    /// Updates the true [`SlotNumber`] and the [`VisibleSlotNumber`].
    ///
    /// This method is used in tests.
    #[cfg(test)]
    pub fn update_version(&mut self, visible_slot_number: u64) {
        self.visible_slot_num = VisibleSlotNumber::new_dangerous(visible_slot_number);
    }

    #[cfg(feature = "native")]
    /// Returns the list of all changes contained in the state checkpoint.
    pub fn changes(&mut self) -> ChangeSet {
        self.delta.changes()
    }

    /// Directly apply a set of changes to the state checkpoint. This method should generally *not* be used
    /// during normal execution, since changes should happen through `StateValue` types which
    /// use the UniversalStateAccessor API. It is primarily intended for use in the sequencer, which has to manage
    /// its own state.
    // This TODO is not a security risk, it is used only in sequencer as intended.
    // TODO: Remove this method if we stop using `StateCheckpoint` in the sequencer
    #[cfg(feature = "native")]
    pub fn apply_changes(&mut self, changeset: ChangeSet) {
        for ((key, namespace), value) in changeset.changes {
            if let Some(value) = value {
                self.delta.set(namespace, &key, value);
            } else {
                self.delta.delete(namespace, &key);
            }
        }
    }

    /// Directly apply a set of changes to the state checkpoint. This method should generally *not* be used
    /// during normal execution, since changes should happen through `StateValue` types which
    /// use the UniversalStateAccessor API. It is primarily intended for use in the sequencer, which has to manage
    /// its own state.
    // This TODO is not a security risk, it is used only in sequencer as intended.
    // TODO: Remove this method if we stop using `StateCheckpoint` in the sequencer
    #[cfg(feature = "native")]
    pub fn apply_tx_changes(&mut self, changeset: TxChangeSet) {
        for ((key, namespace), value) in changeset.writes {
            if let Some(value) = value {
                self.set_value(namespace, &key, value);
            } else {
                self.delete_value(namespace, &key);
            }
        }
    }

    /// Advances the visible slot number and rollup height.
    #[cfg(feature = "native")]
    pub fn advance_visible_slot_number(&mut self, advance: std::num::NonZero<u8>) {
        self.visible_slot_num.advance(advance.get().into());
        self.rollup_height.incr();
    }
}

impl<S: Spec> VersionReader for StateCheckpoint<S> {
    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.visible_slot_num
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.visible_slot_num.as_true()
    }

    fn rollup_height_to_access(&self) -> RollupHeight {
        self.rollup_height
    }
}

impl<S: Spec> UniversalStateAccessor for StateCheckpoint<S> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        self.delta.get_size(namespace, key, metric)
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        self.delta.get(namespace, key, metric)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.delta.set(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.delta.delete(namespace, key);
    }
}

#[cfg(feature = "native")]
pub mod native {
    use sov_metrics::StateAccessMetric;
    use sov_state::{SlotKey, SlotValue};

    use crate::state::accessors::UniversalStateAccessor;
    use crate::{Spec, StateCheckpoint};

    impl<S: Spec> StateCheckpoint<S> {
        /// Returns a handler for the accessory state (non-JMT state).
        ///
        /// You can use this method when calling getters and setters on accessory
        /// state containers, like AccessoryStateMap.
        pub fn accessory_state(&mut self) -> AccessoryStateCheckpoint<S> {
            AccessoryStateCheckpoint { checkpoint: self }
        }
    }

    /// A wrapper over [`crate::StateCheckpoint`] that only allows access to the accessory
    /// state (non-JMT state).
    pub struct AccessoryStateCheckpoint<'a, S: Spec> {
        pub(in crate::state) checkpoint: &'a mut StateCheckpoint<S>,
    }

    impl<S: Spec> UniversalStateAccessor for AccessoryStateCheckpoint<'_, S> {
        fn get_size(
            &mut self,
            namespace: sov_state::Namespace,
            key: &SlotKey,
            metric: &mut StateAccessMetric,
        ) -> Option<u32> {
            self.checkpoint.get_size(namespace, key, metric)
        }

        fn get_value(
            &mut self,
            namespace: sov_state::Namespace,
            key: &SlotKey,
            metric: &mut StateAccessMetric,
        ) -> Option<SlotValue> {
            self.checkpoint.get_value(namespace, key, metric)
        }

        fn set_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey, value: SlotValue) {
            self.checkpoint.set_value(namespace, key, value);
        }

        fn delete_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) {
            self.checkpoint.delete_value(namespace, key);
        }
    }
}

impl<S: Spec> GasMeter for StateCheckpoint<S> {
    type Spec = S;
}

impl<S: Spec> PerBlockCache for StateCheckpoint<S> {
    fn get_cached<T: 'static + Send + Sync>(&self, slot_key: Option<SlotKey>) -> Option<&T> {
        if let CacheLookup::Hit(value) = self.cache.get::<T>(slot_key) {
            value
        } else {
            None
        }
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(
        &mut self,
        slot_key: Option<SlotKey>,
        value: T,
    ) {
        self.cache.set(slot_key, value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self, slot_key: Option<SlotKey>) {
        self.cache.delete::<T>(slot_key);
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.cache.update_with(other);
        self.cache.prune();
    }
}
