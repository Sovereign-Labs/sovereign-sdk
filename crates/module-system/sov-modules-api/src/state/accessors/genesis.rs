use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{EventContainer, SlotKey, SlotValue, TypeErasedEvent};

use super::checkpoints::StateCheckpoint;
use super::temp_cache::{BorshSerializedSize, CacheLookup, TempCache};
use super::UniversalStateAccessor;
use crate::capabilities::RollupHeight;
use crate::state::traits::PerBlockCache;
use crate::{GasMeter, Genesis, PrivilegedKernelAccessor, Spec, VersionReader};

/// A special state accessor which can only be used at genesis.
/// Since genesis is unproven, this state accessor may read and write to every namespace, and it is not metered.
pub struct GenesisStateAccessor<'a, S: Spec> {
    checkpoint: &'a mut StateCheckpoint<S>,
    pub(super) events: Vec<TypeErasedEvent>,
    pub(super) cache: TempCache,
}

impl<S: Spec> StateCheckpoint<S> {
    /// Produces an unmetered [`GenesisStateAccessor`] from a [`StateCheckpoint`] for genesis.
    pub fn to_genesis_state_accessor<G: Genesis>(
        &mut self,
        // This argument prevents this method from being called outside of genesis.
        _config: &G::Config,
    ) -> GenesisStateAccessor<S> {
        GenesisStateAccessor {
            checkpoint: self,
            events: Vec::default(),
            cache: TempCache::new(),
        }
    }
}

impl<S: Spec> PrivilegedKernelAccessor for GenesisStateAccessor<'_, S> {
    fn true_slot_number(&self) -> SlotNumber {
        SlotNumber::GENESIS
    }
}

impl<S: Spec> VersionReader for GenesisStateAccessor<'_, S> {
    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        VisibleSlotNumber::GENESIS
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        VisibleSlotNumber::GENESIS.as_true()
    }

    fn rollup_height_to_access(&self) -> RollupHeight {
        RollupHeight::GENESIS
    }
}

impl<S: Spec> UniversalStateAccessor for GenesisStateAccessor<'_, S> {
    fn get_size(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<u32> {
        self.checkpoint.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.checkpoint.get_value(namespace, key)
    }

    fn set_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey, value: SlotValue) {
        self.checkpoint.set_value(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) {
        self.checkpoint.delete_value(namespace, key);
    }
}

impl<S: Spec> GasMeter for GenesisStateAccessor<'_, S> {
    type Spec = S;
}

impl<S: Spec> GenesisStateAccessor<'_, S> {
    /// Extracts all typed events from this working set.
    pub fn take_events(&mut self) -> Vec<TypeErasedEvent> {
        core::mem::take(&mut self.events)
    }

    /// Extracts a typed event at index `index`
    pub fn take_event(&mut self, index: usize) -> Option<TypeErasedEvent> {
        if index < self.events.len() {
            Some(self.events.remove(index))
        } else {
            None
        }
    }

    /// Returns an immutable map of all typed events that have been previously
    /// written to this working set.
    #[must_use]
    pub fn events(&self) -> &[TypeErasedEvent] {
        &self.events
    }
}

impl<S: Spec> EventContainer for GenesisStateAccessor<'_, S> {
    fn add_event<E: 'static + core::marker::Send>(&mut self, event_key: &str, event: E) {
        self.events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        self.events.push(event);
    }
}

use crate::GenesisState;
impl<S: Spec> GenesisState<S> for GenesisStateAccessor<'_, S> {}

impl<S: Spec> PerBlockCache for GenesisStateAccessor<'_, S> {
    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.cache.set(value);
    }
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        if let CacheLookup::Hit(value) = self.cache.get::<T>() {
            value
        } else {
            None
        }
    }
    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.cache.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.cache.update_with(other);
        self.cache.prune(); // Since there's no other cache under the Genesis state, we can prune `None` entries
    }
}
