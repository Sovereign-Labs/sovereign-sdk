use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{IsValueCached, Namespace, SlotKey, SlotValue, StateAccesses, Storage};
use tracing::trace;

use super::internals::{AccessoryDelta, Delta};
use super::temp_cache::{CacheLookup, TempCache};
use super::{BootstrapWorkingSet, BorshSerializedSize, UniversalStateAccessor};
use crate::capabilities::{Kernel, RollupHeight};
use crate::state::traits::PerBlockCache;
use crate::{GasMeter, Spec, VersionReader};
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
        }
    }

    /// Returns a reference to the storage underlying the state checkpoint.
    #[cfg(feature = "native")]
    pub fn storage(&self) -> &S::Storage {
        self.delta.inner()
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
        let mut delta = Delta::with_witness(inner, witness);
        let mut bootstrap_state = BootstrapWorkingSet { inner: &mut delta };

        let visible_slot_num = kernel.next_visible_slot_number(&mut bootstrap_state);
        trace!(%visible_slot_num, "Initializing a `StateCheckpoint`");
        let rollup_height = kernel.rollup_height(&mut bootstrap_state);
        Self {
            delta,
            visible_slot_num,
            rollup_height,
            cache: TempCache::new(),
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
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        self.delta.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.delta.get(namespace, key)
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
        fn get_size(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<u32> {
            self.checkpoint.get_size(namespace, key)
        }

        fn get_value(
            &mut self,
            namespace: sov_state::Namespace,
            key: &SlotKey,
        ) -> Option<SlotValue> {
            self.checkpoint.get_value(namespace, key)
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
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        if let CacheLookup::Hit(value) = self.cache.get::<T>() {
            value
        } else {
            None
        }
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.cache.set(value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.cache.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.cache.update_with(other);
        self.cache.prune();
    }
}
