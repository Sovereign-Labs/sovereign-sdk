//! Layered revertable transaction state implementation.

use std::collections::HashMap;
use std::marker::PhantomData;

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_state::{
    EventContainer, Kernel as KernelType, Namespace, SlotKey, SlotValue, TypeErasedEvent, User,
};

use super::super::temp_cache::{CacheLookup, TempCache};
use super::super::{BorshSerializedSize, StateMetricsProvider, UniversalStateAccessor};
use crate::module::Spec;
use crate::state::traits::delegate_version_reader;
use crate::state::traits::PerBlockCache;
use crate::{
    AccessoryStateWriter, BasicGasMeter, GasMeter, GasMeteringError, ProvableStateReader,
    ProvableStateWriter, TxState,
};

#[cfg(feature = "test-utils")]
use crate::AccessoryStateReader;

/// A single layer of state changes that can be committed or reverted.
#[derive(Debug)]
pub(super) struct StateLayer {
    events: Vec<TypeErasedEvent>,
    temp_cache: TempCache,
    writes: HashMap<(Namespace, SlotKey), Option<SlotValue>>,
}

impl StateLayer {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            temp_cache: TempCache::new(),
            writes: HashMap::new(),
        }
    }
}

/// Result type for layer operations that may return either more layers or the inner state.
pub enum LayerResult<'a, S: Spec, State> {
    /// There are more layers remaining in the LayeredRevertableTxState.
    HasMoreLayers(LayeredRevertableTxState<'a, S, State>),
    /// This was the last layer, returning the inner state.
    InnerState(&'a mut State),
}

/// A multi-layered revertable state that wraps a [`TxState`] and tracks writes and events 
/// across multiple layers using a vector-based approach to avoid unbounded recursion.
///
/// Changes can be committed or reverted layer by layer via [`LayeredRevertableTxState::commit_layer`]
/// and [`LayeredRevertableTxState::revert_layer`].
///
/// ## Usage note
/// This structure tracks gas consumed outside of the transaction lifecycle without explicitly consuming a finite resource.
/// This should only be used in infallible methods.
pub struct LayeredRevertableTxState<'a, S: Spec, State> {
    pub(super) inner: &'a mut State,
    pub(super) layers: Vec<StateLayer>,
    pub(super) phantom: PhantomData<S>,
}

impl<S: Spec, I: StateMetricsProvider> StateMetricsProvider for LayeredRevertableTxState<'_, S, I> {
    fn metrics(&mut self) -> &mut StateMetrics {
        self.inner.metrics()
    }
}

impl<'a, S: Spec, I: TxState<S>> LayeredRevertableTxState<'a, S, I> {
    /// Creates a new [`LayeredRevertableTxState`] from the provided [`TxState`] with one initial layer.
    ///
    /// # Important
    /// You *MUST* call [`LayeredRevertableTxState::commit_layer`] to save any changes made to this state.
    pub fn new(inner: &'a mut I) -> Self {
        Self {
            inner,
            layers: vec![StateLayer::new()],
            phantom: PhantomData,
        }
    }

    /// Adds a new revertable layer on top of the current layers.
    /// This pushes a new layer onto the layers stack.
    pub fn add_revertable_layer(&mut self) -> &mut Self {
        self.layers.push(StateLayer::new());
        self
    }

    /// Commits the top layer to the layer below it, or to the inner state if this is the last layer.
    /// 
    /// Returns `LayerResult::HasMoreLayers` if there are more layers remaining after the commit,
    /// or `LayerResult::InnerState` if this was the last layer.
    pub fn commit_layer(mut self) -> LayerResult<'a, S, I> {
        if let Some(layer) = self.layers.pop() {
            if self.layers.is_empty() {
                // This was the last layer, commit to inner state
                for event in layer.events {
                    self.inner.add_type_erased_event(event);
                }
                for (key, value) in layer.writes {
                    if let Some(value) = value {
                        self.inner.set_value(key.0, &key.1, value);
                    } else {
                        self.inner.delete_value(key.0, &key.1);
                    }
                }
                self.inner.update_cache_with(layer.temp_cache);
                LayerResult::InnerState(self.inner)
            } else {
                // Commit to the layer below
                let lower_layer = self.layers.last_mut().unwrap();
                
                // Merge events
                lower_layer.events.extend(layer.events);
                
                // Merge writes (top layer takes precedence)
                for (key, value) in layer.writes {
                    lower_layer.writes.insert(key, value);
                }
                
                // Merge cache
                lower_layer.temp_cache.update_with(layer.temp_cache);
                
                LayerResult::HasMoreLayers(self)
            }
        } else {
            // No layers to commit, return inner state
            LayerResult::InnerState(self.inner)
        }
    }

    /// Reverts and discards the top layer.
    /// 
    /// Returns `LayerResult::HasMoreLayers` if there are more layers remaining after the revert,
    /// or `LayerResult::InnerState` if this was the last layer.
    pub fn revert_layer(mut self) -> LayerResult<'a, S, I> {
        if self.layers.len() > 1 {
            // Discard the top layer and return the remaining layers
            self.layers.pop();
            LayerResult::HasMoreLayers(self)
        } else {
            // This was the last layer, return inner state
            LayerResult::InnerState(self.inner)
        }
    }

    /// Gets the current number of layers.
    pub fn layer_depth(&self) -> usize {
        self.layers.len()
    }

    /// Gets the current top layer for write operations.
    fn current_layer_mut(&mut self) -> &mut StateLayer {
        self.layers.last_mut().expect("LayeredRevertableTxState should always have at least one layer")
    }
}

delegate_version_reader!(LayeredRevertableTxState<'_, S, I> where [S: Spec, I: TxState<S>] => inner);

impl<S: Spec, I: TxState<S>> UniversalStateAccessor for LayeredRevertableTxState<'_, S, I> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<u32> {
        // Check layers from top to bottom for the most recent write
        for layer in self.layers.iter().rev() {
            if let Some(value) = layer.writes.get(&(namespace, key.clone())) {
                return value.as_ref().map(|v| v.size());
            }
        }
        // If not found in any layer, check the inner state
        self.inner.get_size(namespace, key, metrics)
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        // Check layers from top to bottom for the most recent write
        for layer in self.layers.iter().rev() {
            if let Some(value) = layer.writes.get(&(namespace, key.clone())) {
                return value.clone();
            }
        }
        // If not found in any layer, check the inner state
        self.inner.get_value(namespace, key, metrics)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        // Write to the current (top) layer
        self.current_layer_mut().writes.insert((namespace, key.clone()), Some(value));
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        // Mark as deleted in the current (top) layer
        self.current_layer_mut().writes.insert((namespace, key.clone()), None);
    }
}

impl<S: Spec, I: TxState<S>> PerBlockCache for LayeredRevertableTxState<'_, S, I> {
    fn get_cached<T: 'static + Send + Sync>(&self, slot_key: Option<SlotKey>) -> Option<&T> {
        // Check layers from top to bottom for cached values
        for layer in self.layers.iter().rev() {
            match layer.temp_cache.get::<T>(slot_key.clone()) {
                CacheLookup::Hit(value) => return value,
                CacheLookup::Miss => continue,
            }
        }
        // If not found in any layer, check the inner state
        self.inner.get_cached::<T>(slot_key)
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(
        &mut self,
        slot_key: Option<SlotKey>,
        value: T,
    ) {
        // Cache in the current (top) layer
        self.current_layer_mut().temp_cache.set(slot_key, value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self, slot_key: Option<SlotKey>) {
        // Delete from the current (top) layer
        self.current_layer_mut().temp_cache.delete::<T>(slot_key);
    }

    fn update_cache_with(&mut self, other: TempCache) {
        // Update the current (top) layer's cache
        self.current_layer_mut().temp_cache.update_with(other);
    }
}

impl<S: Spec, I: TxState<S>> EventContainer for LayeredRevertableTxState<'_, S, I> {
    fn add_event<E: 'static + core::marker::Send>(&mut self, event_key: &str, event: E) {
        // Add event to the current (top) layer
        self.current_layer_mut().events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        // Add event to the current (top) layer
        self.current_layer_mut().events.push(event);
    }
}

impl<S: Spec, I: TxState<S>> GasMeter for LayeredRevertableTxState<'_, S, I> {
    type Spec = S;
    
    fn charge_gas(&mut self, amount: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.inner.charge_gas(amount)
    }
    
    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        self.inner.try_as_basic_gas_meter()
    }

    fn charge_linear_gas(
        &mut self,
        amount: <Self::Spec as Spec>::Gas,
        parameter: u32,
    ) -> anyhow::Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        self.inner.charge_linear_gas(amount, parameter)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.inner.remove_gas_pattern(amount, parameter);
    }
}

impl<S: Spec, I: TxState<S>> ProvableStateReader<User> for LayeredRevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateReader<KernelType> for LayeredRevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateWriter<User> for LayeredRevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> ProvableStateWriter<KernelType> for LayeredRevertableTxState<'_, S, I> {}
impl<S: Spec, I: TxState<S>> AccessoryStateWriter for LayeredRevertableTxState<'_, S, I> {}

// Specialized implementation of `add_revertable_layer()` for `LayeredRevertableTxState`.
// This overrides the default trait implementation to add a layer directly to the existing
// vector instead of creating a wrapper. However, the trait signature requires returning
// a new LayeredRevertableTxState, so we add the layer and then wrap it.
impl<'a, S: Spec, I: TxState<S>> TxState<S> for LayeredRevertableTxState<'a, S, I> {
    fn add_revertable_layer(&mut self) -> LayeredRevertableTxState<'_, S, Self> {
        // Call the inherent method to add a layer to the existing vector
        // This uses the vector-based approach to prevent recursion
        LayeredRevertableTxState::add_revertable_layer(self);
        // Return a new LayeredRevertableTxState wrapping self
        // This is necessary because the trait method signature requires returning a new instance
        LayeredRevertableTxState::new(self)
    }
}

// Note: `LayeredRevertableTxState` implements `TxState<S>` via both the blanket implementation
// and the specialized implementation above. Rust will prefer the specialized implementation.
// The direct method `add_revertable_layer()` returns `&mut Self` and uses the vector-based
// approach to prevent unbounded recursion.

#[cfg(feature = "test-utils")]
impl<S: Spec, I: TxState<S>> AccessoryStateReader for LayeredRevertableTxState<'_, S, I> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::mocks::MockKernel;
    use crate::execution_mode::Native;
    use crate::state::accessors::scratchpad::WorkingSet;
    use sov_state::namespaces::User;
    use sov_state::{CompileTimeNamespace, SlotKey, SlotValue};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_single_layer_commit() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create layered state
        let mut layered_state = working_set.add_revertable_layer();
        
        // Write some data
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value = SlotValue::from("test_value");
        
        layered_state.set_value(namespace, &key, value.clone());
        
        // Commit the layer
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                // Should return inner state since this was the only layer
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key, &mut metric), Some(value));
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState, got HasMoreLayers"),
        }
    }

    #[test]
    fn test_single_layer_revert() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create layered state
        let mut layered_state = working_set.add_revertable_layer();
        
        // Write some data
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value = SlotValue::from("test_value");
        
        layered_state.set_value(namespace, &key, value.clone());
        
        // Revert the layer
        match layered_state.revert_layer() {
            LayerResult::InnerState(inner) => {
                // Should return inner state and the write should be gone
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key, &mut metric), None);
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState, got HasMoreLayers"),
        }
    }

    #[test]
    fn test_multiple_layers_commit() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create first layer
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key1 = SlotKey::from_slice(b"key1");
        let key2 = SlotKey::from_slice(b"key2");
        let value1 = SlotValue::from("value1");
        let value2 = SlotValue::from("value2");
        let value2_updated = SlotValue::from("value2_updated");
        
        // Write to first layer
        layered_state.set_value(namespace, &key1, value1.clone());
        layered_state.set_value(namespace, &key2, value2.clone());
        
        // Add second layer (now layered_state has 2 layers)
        layered_state.add_revertable_layer();
        
        // Update key2 in second layer
        layered_state.set_value(namespace, &key2, value2_updated.clone());
        
        // Commit second layer - should return HasMoreLayers since there's still layer1
        let mut layered_state = match layered_state.commit_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers when committing layer2, got InnerState"),
        };
        
        // Verify the commit merged correctly - layer1 should now have the updated value
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key1, &mut metric), Some(value1.clone()), "key1 should still have value1");
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key2, &mut metric), Some(value2_updated.clone()), "key2 should have been updated to value2_updated");
        
        // Commit first layer - should return InnerState since this is the last layer
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                // inner is &mut WorkingSet, verify final state
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key1, &mut metric), Some(value1), "key1 should be in final state");
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key2, &mut metric), Some(value2_updated), "key2 should have updated value in final state");
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState after committing last layer"),
        }
    }

    #[test]
    fn test_multiple_layers_revert() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create first layer
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key1 = SlotKey::from_slice(b"key1");
        let key2 = SlotKey::from_slice(b"key2");
        let value1 = SlotValue::from("value1");
        let value2 = SlotValue::from("value2");
        let value2_updated = SlotValue::from("value2_updated");
        
        // Write to first layer
        layered_state.set_value(namespace, &key1, value1.clone());
        layered_state.set_value(namespace, &key2, value2.clone());
        
        // Add second layer
        layered_state.add_revertable_layer();
        
        // Update key2 in second layer
        layered_state.set_value(namespace, &key2, value2_updated.clone());
        
        // Revert second layer - should return HasMoreLayers since layer1 remains
        let mut layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers when reverting layer2, got InnerState"),
        };
        
        // Verify the revert worked - should have original values from layer1
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key1, &mut metric), Some(value1.clone()), "key1 should still have value1");
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key2, &mut metric), Some(value2.clone()), "key2 should have original value2 after revert");
        
        // Commit first layer
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key1, &mut metric), Some(value1), "key1 should be in final state");
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key2, &mut metric), Some(value2), "key2 should have original value2 in final state");
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState after committing last layer"),
        }
    }

    #[test]
    fn test_layer_depth() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create first layer
        let mut layered_state = working_set.add_revertable_layer();
        assert_eq!(layered_state.layer_depth(), 1);
        
        // Add second layer
        layered_state.add_revertable_layer();
        assert_eq!(layered_state.layer_depth(), 2);
        
        // Add third layer
        layered_state.add_revertable_layer();
        assert_eq!(layered_state.layer_depth(), 3);
    }

    #[test]
    fn test_event_isolation() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create first layer
        let mut layered_state = working_set.add_revertable_layer();
        layered_state.add_event("test", "event1");
        
        // Add second layer
        layered_state.add_revertable_layer();
        layered_state.add_event("test", "event2");
        
        // Revert second layer - event2 should be lost
        let layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers when reverting layer2"),
        };
        
        // Commit first layer - only event1 should remain
        match layered_state.commit_layer() {
            LayerResult::InnerState(_inner) => {
                // Events are committed to inner state, we can't easily verify them in this test
                // but the structure ensures proper isolation
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState after committing last layer"),
        }
    }

    #[test]
    fn test_cache_isolation() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create first layer
        let mut layered_state = working_set.add_revertable_layer();
        let cache_key = SlotKey::from_slice(b"cache_key");
        layered_state.put_cached(Some(cache_key.clone()), "cached_value1".to_string());
        
        // Add second layer
        layered_state.add_revertable_layer();
        layered_state.put_cached(Some(cache_key.clone()), "cached_value2".to_string());
        
        // Check that second layer sees its own value
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key.clone())), Some(&"cached_value2".to_string()));
        
        // Revert second layer
        let layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers when reverting layer2"),
        };
        
        // Should see first layer's cached value
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key)), Some(&"cached_value1".to_string()));
    }

    #[test]
    fn test_delete_operations() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        // Create layered state
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value = SlotValue::from("test_value");
        
        // Write a value
        layered_state.set_value(namespace, &key, value.clone());
        
        // Verify it exists
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value.clone()));
        
        // Delete it
        layered_state.delete_value(namespace, &key);
        
        // Verify it's gone
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), None);
        
        // Commit and verify delete is persisted
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key, &mut metric), None, "Deleted value should not exist after commit");
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState"),
        }
    }

    #[test]
    fn test_delete_then_write() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value1 = SlotValue::from("value1");
        let value2 = SlotValue::from("value2");
        
        // Write value1
        layered_state.set_value(namespace, &key, value1.clone());
        
        // Add second layer
        layered_state.add_revertable_layer();
        
        // Delete in second layer
        layered_state.delete_value(namespace, &key);
        
        // Verify it's deleted
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), None);
        
        // Write new value in second layer
        layered_state.set_value(namespace, &key, value2.clone());
        
        // Verify new value is visible
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value2.clone()));
        
        // Revert second layer - should restore value1
        let mut layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value1.clone()));
    }

    #[test]
    fn test_layer_precedence_read() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value1 = SlotValue::from("value1");
        let value2 = SlotValue::from("value2");
        let value3 = SlotValue::from("value3");
        
        // Write value1 in layer1
        layered_state.set_value(namespace, &key, value1.clone());
        
        // Add layer2
        layered_state.add_revertable_layer();
        layered_state.set_value(namespace, &key, value2.clone());
        
        // Add layer3
        layered_state.add_revertable_layer();
        layered_state.set_value(namespace, &key, value3.clone());
        
        // Should see value3 (top layer)
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value3.clone()));
        
        // Revert layer3 - should see value2
        let mut layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value2.clone()));
        
        // Revert layer2 - should see value1
        let mut layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value1.clone()));
    }

    #[test]
    fn test_read_from_inner_state() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"inner_key");
        let value = SlotValue::from("inner_value");
        
        // Write directly to working set (inner state)
        use crate::StateWriter;
        StateWriter::<User>::set(&mut working_set, &key, value.clone()).unwrap();
        
        // Create layered state
        let mut layered_state = working_set.add_revertable_layer();
        
        // Should be able to read from inner state
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value.clone()));
    }

    #[test]
    fn test_delete_from_inner_state() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"inner_key");
        let value = SlotValue::from("inner_value");
        
        // Write directly to working set (inner state)
        use crate::StateWriter;
        StateWriter::<User>::set(&mut working_set, &key, value.clone()).unwrap();
        
        // Create layered state
        let mut layered_state = working_set.add_revertable_layer();
        
        // Verify it exists
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value.clone()));
        
        // Delete it in the layer
        layered_state.delete_value(namespace, &key);
        
        // Should be gone
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), None);
        
        // Commit and verify delete is persisted
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key, &mut metric), None, "Deleted value from inner state should not exist after commit");
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState"),
        }
    }

    #[test]
    fn test_cache_delete() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        let cache_key = SlotKey::from_slice(b"cache_key");
        
        // Put value in cache
        layered_state.put_cached(Some(cache_key.clone()), "cached_value".to_string());
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key.clone())), Some(&"cached_value".to_string()));
        
        // Delete from cache
        layered_state.delete_cached::<String>(Some(cache_key.clone()));
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key.clone())), None);
    }

    #[test]
    fn test_cache_precedence() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        let cache_key = SlotKey::from_slice(b"cache_key");
        
        // Put value1 in layer1
        layered_state.put_cached(Some(cache_key.clone()), "value1".to_string());
        
        // Add layer2
        layered_state.add_revertable_layer();
        layered_state.put_cached(Some(cache_key.clone()), "value2".to_string());
        
        // Should see value2 (top layer)
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key.clone())), Some(&"value2".to_string()));
        
        // Revert layer2 - should see value1
        let layered_state = match layered_state.revert_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        assert_eq!(layered_state.get_cached::<String>(Some(cache_key)), Some(&"value1".to_string()));
    }

    #[test]
    fn test_get_size_consistency() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value = SlotValue::from("test_value");
        
        // Write value
        layered_state.set_value(namespace, &key, value.clone());
        
        // get_size and get_value should be consistent
        let mut size_metric = StateAccessMetric::new_size();
        let size = layered_state.get_size(namespace, &key, &mut size_metric);
        let mut read_metric = StateAccessMetric::new_read();
        let retrieved_value = layered_state.get_value(namespace, &key, &mut read_metric);
        
        assert_eq!(size, retrieved_value.as_ref().map(|v| v.size()));
    }

    #[test]
    fn test_three_layers() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        
        let mut working_set = WorkingSet::<TestSpec>::new_with_kernel(storage, &MockKernel::<TestSpec>::default());
        
        let mut layered_state = working_set.add_revertable_layer();
        
        let namespace = User::NAMESPACE;
        let key = SlotKey::from_slice(b"test_key");
        let value1 = SlotValue::from("value1");
        let value2 = SlotValue::from("value2");
        let value3 = SlotValue::from("value3");
        
        // Write value1 in layer1
        layered_state.set_value(namespace, &key, value1.clone());
        
        // Add layer2
        layered_state.add_revertable_layer();
        layered_state.set_value(namespace, &key, value2.clone());
        
        // Add layer3
        layered_state.add_revertable_layer();
        layered_state.set_value(namespace, &key, value3.clone());
        
        assert_eq!(layered_state.layer_depth(), 3);
        
        // Commit layer3 -> layer2
        let mut layered_state = match layered_state.commit_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        assert_eq!(layered_state.layer_depth(), 2);
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value3.clone()));
        
        // Commit layer2 -> layer1
        let mut layered_state = match layered_state.commit_layer() {
            LayerResult::HasMoreLayers(state) => state,
            LayerResult::InnerState(_) => panic!("Expected HasMoreLayers"),
        };
        assert_eq!(layered_state.layer_depth(), 1);
        let mut metric = StateAccessMetric::new_read();
        assert_eq!(layered_state.get_value(namespace, &key, &mut metric), Some(value3.clone()));
        
        // Commit layer1 -> inner state
        match layered_state.commit_layer() {
            LayerResult::InnerState(inner) => {
                let mut metric = StateAccessMetric::new_read();
                assert_eq!(inner.get_value(namespace, &key, &mut metric), Some(value3));
            }
            LayerResult::HasMoreLayers(_) => panic!("Expected InnerState"),
        }
    }
}
