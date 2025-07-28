use std::collections::HashMap;
use std::sync::Arc;

use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{
    namespaces, EventContainer, Namespace, NativeStorage, ProvableStorageCache, SlotKey, SlotValue,
    Storage, TypeErasedEvent,
};

use super::temp_cache::{CacheLookup, TempCache};
use super::{BorshSerializedSize, StateCheckpoint, UniversalStateAccessor};
use crate::capabilities::{KernelWithSlotMapping, RollupHeight};
use crate::gas::GasArray;
use crate::state::traits::PerBlockCache;
use crate::{Gas, GasMeter, GetGasPrice, Spec, VersionReader};

fn get_slot_number(visible_slot_number: Option<VisibleSlotNumber>) -> Option<SlotNumber> {
    // This TODO is not a security risk.
    // TODO: This is an ugly hack to work around the dual use of self.visible_slot_number.
    // We use it inside `VersionReader` to determine what state the accessor is allowed to access (which requires it to be set)
    // but also here to determine what state version to query *from disk*. Unfortunately, those two numbers don't always agree *during initialization*,
    // so we have to use u64::MAX as a marker value to say "the accessor should be allowed to access any value and should use the latest info from storage as necessary."
    // We should separate out the notion of permissions from the notion of what state version to query from disk.
    if let Some(num) = visible_slot_number {
        if num.get() != u64::MAX {
            Some(num.as_true())
        } else {
            None
        }
    } else {
        None
    }
}

impl<S: Spec> UniversalStateAccessor for ApiStateAccessor<S> {
    fn get_size(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<u32> {
        match namespace {
            Namespace::User => self.user_cache.get_size_or_fetch_historical(
                key,
                &self.storage,
                &self.witness,
                self.safe_true_slot_number_to_use,
            ),
            Namespace::Kernel => self.kernel_cache.get_size_or_fetch_historical(
                key,
                &self.storage,
                &self.witness,
                get_slot_number(self.visible_slot_number),
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(Some(value)) => Some(value.size()),
                Some(None) => None,
                None => {
                    let val = self
                        .storage
                        .get_accessory(key, self.safe_true_slot_number_to_use);
                    val.map(|v| v.size())
                }
            },
        }
    }

    fn get_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) -> Option<SlotValue> {
        match namespace {
            Namespace::User => self.user_cache.get_or_fetch_historical(
                key,
                &self.storage,
                &self.witness,
                self.safe_true_slot_number_to_use,
            ),
            Namespace::Kernel => self.kernel_cache.get_or_fetch_historical(
                key,
                &self.storage,
                &self.witness,
                get_slot_number(self.visible_slot_number),
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(Some(value)) => Some(value),
                Some(None) => None,
                None => self
                    .storage
                    .get_accessory(key, self.safe_true_slot_number_to_use),
            },
        }
    }

    fn set_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey, value: SlotValue) {
        match namespace {
            Namespace::User => self.user_cache.set(key, value),
            Namespace::Kernel => self.kernel_cache.set(key, value),
            Namespace::Accessory => {
                self.accessory_writes.insert(key.clone(), Some(value));
            }
        }
    }

    fn delete_value(&mut self, namespace: sov_state::Namespace, key: &SlotKey) {
        match namespace {
            Namespace::User => self.user_cache.delete(key),
            Namespace::Kernel => self.kernel_cache.delete(key),
            Namespace::Accessory => {
                self.accessory_writes.remove(key);
            }
        }
    }
}

#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StateToAccess {
    RollupHeight(RollupHeight),
    TrueSlotNumber(
        SlotNumber,
        // We always cache the rollup height for the requested slot number during initialization. However,
        // it can be `None` while initialization is still in progress.
        Option<RollupHeight>,
    ),
}

/// A [`crate::StateReaderAndWriter`] designed for use within REST APIs and JSON-RPC.
///
/// It can read and write accessory data as well as "user" and "kernel" data.
#[derive(derive_more::Debug)]
pub struct ApiStateAccessor<S: Spec> {
    #[debug(skip)]
    storage: S::Storage,
    #[debug(skip)]
    witness: <<S as Spec>::Storage as Storage>::Witness,
    events: Vec<TypeErasedEvent>,
    gas_price: <S::Gas as Gas>::Price,
    kernel_cache: ProvableStorageCache<namespaces::Kernel>,
    user_cache: ProvableStorageCache<namespaces::User>,
    accessory_writes: HashMap<SlotKey, Option<SlotValue>>,
    temp_cache: TempCache,
    #[debug(skip)]
    kernel: Arc<dyn KernelWithSlotMapping<S>>,
    // The state requested by the user - either a `RollupHeight` or a `SlotNumber`
    state_to_access: StateToAccess,
    // The visible slot number is always stored in the accessor by the time initialization is complete.
    // Unfortunately, we need to run one query using the accessor in order to determine the correct visible slot number,
    // so we can't make this field non-optional.
    visible_slot_number: Option<VisibleSlotNumber>,
    // The true slot number to use for user/accessory queries. This need not correspond exactly the the accessor's
    // rollup height (if present) but it must be older than the N+1th rollup height.
    //
    // Suppose we have the following rollup heights:
    //  height:           1 -> 2 -> 3 -> 4 -> 5
    //  true_slot_number: 1 -> 4 -> 7 -> 10 -> 13
    //
    // Then a safe true slot number to use for user/accessory queries with rollup height 4 is 7, 8, 9, or 10 since all of these numbers
    // will cause any state *before* 4 to be visible. (Assume that the state checkpoint the accessor is based on contains all the state *of* 4 in memory)
    safe_true_slot_number_to_use: Option<SlotNumber>,
}

impl<S: Spec> PerBlockCache for ApiStateAccessor<S> {
    fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        if let CacheLookup::Hit(v) = self.temp_cache.get::<T>() {
            v
        } else {
            None
        }
    }

    fn put_cached<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        self.temp_cache.set(value);
    }

    fn delete_cached<T: 'static + Send + Sync>(&mut self) {
        self.temp_cache.delete::<T>();
    }

    fn update_cache_with(&mut self, other: TempCache) {
        self.temp_cache.update_with(other);
    }
}

#[cfg(feature = "native")]
const _: () = {
    use sov_state::{NativeStorage, ProvableCompileTimeNamespace, StorageProof};

    use crate::{ProvenStateAccessor, StateReaderAndWriter};

    impl<N, S: Spec> ProvenStateAccessor<N> for ApiStateAccessor<S>
    where
        N: ProvableCompileTimeNamespace,
        S::Storage: NativeStorage,
        ApiStateAccessor<S>: StateReaderAndWriter<N>,
    {
        type Proof = <<S as Spec>::Storage as Storage>::Proof;

        fn get_with_proof(&mut self, key: SlotKey) -> Option<StorageProof<Self::Proof>> {
            // In this case, we need to use an exact slot number rather than an approximate one - otherwise
            // the returned merkle proof will be useless.

            // Temporarily give access to all visible slot numbers for the purpose of retrieving the mapping between true and visible slots.
            // We'll set the value back to more scoped permissions at the end of this function
            let safe_true_slot_number_to_use = self.safe_true_slot_number_to_use;
            self.safe_true_slot_number_to_use = None;
            let slot_num = match self.state_to_access {
                StateToAccess::RollupHeight(rollup_height) => self
                    .kernel
                    .clone()
                    .true_slot_number_at_historical_height(rollup_height, self),
                StateToAccess::TrueSlotNumber(slot_number, _) => Some(slot_number),
            };
            // We set the permissions back to the original value here
            self.safe_true_slot_number_to_use = safe_true_slot_number_to_use;
            let slot_num = slot_num?;

            match self.storage.get_with_proof::<N>(key, Some(slot_num)) {
                Ok(storage_proof) => Some(storage_proof),
                Err(err) => {
                    tracing::debug!(error = ?err, "Error requesting storage proof");
                    None
                }
            }
        }
    }
};

impl<S: Spec> GasMeter for ApiStateAccessor<S> {
    type Spec = S;
}

impl<S: Spec> GetGasPrice for ApiStateAccessor<S> {
    type Spec = S;
    fn gas_price(&self) -> &<S::Gas as Gas>::Price {
        &self.gas_price
    }
}

impl<S: Spec> EventContainer for ApiStateAccessor<S> {
    fn add_event<E: 'static + core::marker::Send>(&mut self, event_key: &str, event: E) {
        self.events.push(TypeErasedEvent::new(event_key, event));
    }

    fn add_type_erased_event(&mut self, event: TypeErasedEvent) {
        self.events.push(event);
    }
}

/// An error that can occur when creating an [`ApiStateAccessor`].
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiStateAccessorError {
    /// The requested height is not accessible.
    #[error("Impossible to get the rollup state at the specified height. Please ensure you have queried the correct height.")]
    HeightNotAccessible,
}

impl<S: Spec + 'static> ApiStateAccessor<S> {
    /// Creates a new [`ApiStateAccessor`] from a [`StateCheckpoint`] with a gas price of zero at the [`StateCheckpoint::rollup_height_to_access`].
    pub fn new(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
    ) -> Self {
        Self::new_with_price_and_state_to_access(
            state_checkpoint,
            kernel,
            StateToAccess::RollupHeight(state_checkpoint.rollup_height_to_access()),
            <S::Gas as Gas>::Price::ZEROED,
        )
        .expect("Creating an ApiStateCheckpoint without specifying a height is infallible")
    }

    /// Creates a new [`ApiStateAccessor`] which queries all state at the provided slot number.
    pub fn new_archival_with_true_slot_number(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        slot_number: SlotNumber,
    ) -> Result<Self, ApiStateAccessorError> {
        Self::build_archival_state(
            state_checkpoint.delta.inner.clone(),
            kernel,
            StateToAccess::TrueSlotNumber(slot_number, None),
        )
    }

    /// Creates a fully initialized [`ApiStateAccessor`] from a [`StateCheckpoint`] and a [`RollupHeight`], if the requested
    /// height is available in storage.
    ///
    /// ## Important
    /// Note that archival state is only available *after* the requested height has been processed by the node. In other words,
    /// state that is *only* soft-confirmed is not available for archival queries.
    pub fn new_archival(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        height: RollupHeight,
    ) -> Result<Self, ApiStateAccessorError> {
        Self::build_archival_state(
            state_checkpoint.delta.inner.clone(),
            kernel,
            StateToAccess::RollupHeight(height),
        )
    }

    /// Creates a new [`ApiStateAccessor`] from a [`StateCheckpoint`] with a gas price of zero at the [`StateCheckpoint::rollup_height_to_access`].
    pub fn new_with_true_slot_number_dangerous(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        slot_number: SlotNumber,
    ) -> Result<Self, ApiStateAccessorError> {
        Self::new_with_price_and_state_to_access(
            state_checkpoint,
            kernel,
            StateToAccess::TrueSlotNumber(slot_number, None),
            <S::Gas as Gas>::Price::ZEROED,
        )
    }

    /// A specialized constructor for use in sov-test-utils. This constructor matches the unusual semantic of our testing framework, which expects to see
    /// the *next* visible slot number with the *current* rollup height in the accessor as soon as the a slot finishes executing.
    ///
    /// In actual execution, this semantic is not needed because the time between the end of slot N and the start of slot N+1 is neglible. Unfortunately,
    /// our tests are written assuming that all assertions will execute during this miniscule instant of time, so we have to accommodate it for now.
    #[cfg(feature = "test-utils")]
    pub fn new_with_price_and_heights(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        rollup_height: RollupHeight,
        visible_slot_number: VisibleSlotNumber,
        gas_price: <S::Gas as Gas>::Price,
    ) -> Result<Self, ApiStateAccessorError> {
        let mut accessor = Self::new_with_price_and_state_to_access(
            state_checkpoint,
            kernel,
            StateToAccess::RollupHeight(rollup_height),
            gas_price,
        )?;
        accessor.visible_slot_number = Some(visible_slot_number);
        Ok(accessor)
    }

    /// Creates a new [`ApiStateAccessor`] that *queries all state at the provided slot number, regardless of whether that slot number is visible*. This should only be used
    /// when the semantics of the call necessitate it. If you're unsure, use [`ApiStateAccessor::new_archival`] and provide a rollup height instead.
    pub fn new_with_price_and_slot_number_dangerous(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        slot_number: SlotNumber,
        gas_price: <S::Gas as Gas>::Price,
    ) -> Result<Self, ApiStateAccessorError> {
        Self::new_with_price_and_state_to_access(
            state_checkpoint,
            kernel,
            StateToAccess::TrueSlotNumber(slot_number, None),
            gas_price,
        )
    }

    /// Creates a new [`ApiStateAccessor`] from a [`StateCheckpoint`] with the provided gas price.
    fn new_with_price_and_state_to_access(
        state_checkpoint: &StateCheckpoint<S>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        state_to_access: StateToAccess,
        gas_price: <S::Gas as Gas>::Price,
    ) -> Result<Self, ApiStateAccessorError> {
        let delta: &super::internals::Delta<<S as Spec>::Storage> = &state_checkpoint.delta;

        let mut out = Self {
            storage: delta.inner.clone(),
            witness: Default::default(),
            gas_price,
            events: Vec::new(),
            temp_cache: TempCache::new(),
            kernel_cache: delta.kernel_cache.clone(),
            user_cache: delta.user_cache.clone(),
            accessory_writes: delta.accessory_writes.clone(),
            kernel: kernel.clone(),
            state_to_access,
            visible_slot_number: None,
            safe_true_slot_number_to_use: None,
        };

        (out.safe_true_slot_number_to_use) = match state_to_access {
            // The current slot may not be stored in the `true_slot_number_at_historical_height` map, so we use the previous slot number.
            StateToAccess::RollupHeight(rollup_height) => {
                // If this is a slot that has already been executed, we know the correct true slot number. Use that.
                kernel.true_slot_number_at_historical_height(rollup_height, &mut out)
            }
            StateToAccess::TrueSlotNumber(slot_number, _) => Some(slot_number),
        };

        if let StateToAccess::TrueSlotNumber(slot_number, _) = state_to_access {
            let Some(rollup_height) =
                kernel.true_slot_number_to_rollup_height(slot_number, &mut out)
            else {
                return Err(ApiStateAccessorError::HeightNotAccessible);
            };
            out.state_to_access = StateToAccess::TrueSlotNumber(slot_number, Some(rollup_height));
        }

        let Some(visible_slot_number) = out.lookup_visible_slot_number() else {
            return Err(ApiStateAccessorError::HeightNotAccessible);
        };
        out.visible_slot_number = Some(visible_slot_number);

        Ok(out)
    }

    fn get_uninitialized_empty_accessor(
        storage: S::Storage,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        state_to_access: StateToAccess,
    ) -> Self {
        Self {
            events: Vec::new(),
            gas_price: <S::Gas as Gas>::Price::ZEROED,
            storage,
            witness: Default::default(),
            kernel_cache: Default::default(),
            user_cache: Default::default(),
            accessory_writes: HashMap::new(),
            temp_cache: TempCache::new(),
            kernel: kernel.clone(),
            state_to_access,
            safe_true_slot_number_to_use: None,
            // We use MAX here to indicate that the accessor should be allowed to access any value. This is necessary because of the fundamental
            // permissions vs state confusion in our current design of VersionReader - see the comment inline in get_cached for more details.
            visible_slot_number: Some(VisibleSlotNumber::MAX),
        }
    }

    fn lookup_visible_slot_number(&mut self) -> Option<VisibleSlotNumber> {
        match self.state_to_access {
            StateToAccess::RollupHeight(rollup_height) => self
                .kernel
                .clone()
                .rollup_height_to_visible_slot_number(rollup_height, self),
            StateToAccess::TrueSlotNumber(slot_number, _) => {
                let visible_slot_number = VisibleSlotNumber::new_dangerous(slot_number.get());
                Some(visible_slot_number)
            }
        }
    }

    fn clear_caches(&mut self) {
        self.kernel_cache = Default::default();
        self.user_cache = Default::default();
        self.accessory_writes = HashMap::new();
    }

    /// Sets the gas price for the accessor.
    pub fn set_gas_price(&mut self, gas_price: <S::Gas as Gas>::Price) {
        self.gas_price = gas_price;
    }

    fn build_archival_state(
        storage: S::Storage,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        height: StateToAccess,
    ) -> Result<Self, ApiStateAccessorError> {
        let latest_true_slot_number = storage.latest_version();
        let mut state =
            ApiStateAccessor::get_uninitialized_empty_accessor(storage, kernel.clone(), height);
        let true_slot_number = match height {
            // If the caller provided a rollup height, find the associated true slot number.
            StateToAccess::RollupHeight(height) => {
                if let Some(true_slot_number) =
                    kernel.true_slot_number_at_historical_height(height, &mut state)
                {
                    true_slot_number
                } else {
                    // There's a tricky case here where the height exists in storage but the true slot number is not available via the kernel yet.
                    // This is because the true slot number mapping is updated at the beginning of the next slot, but the rest of the values are written
                    // at the end of the current slot.
                    //
                    // Since the ApiStateAccessor has empty caches right now, we can check if we're in this case by checking whether the requested height is equal to the current rollup height
                    // as reported by the kernel. (Recall that, since the caches are empty, the "current_rollup_height" value reported by the kernel is the value stored at S::Storage::latest_version.)
                    if height == kernel.current_rollup_height(&mut state) {
                        latest_true_slot_number
                    } else {
                        return Err(ApiStateAccessorError::HeightNotAccessible);
                    }
                }
            }
            StateToAccess::TrueSlotNumber(slot_number, _) => {
                if slot_number > latest_true_slot_number {
                    return Err(ApiStateAccessorError::HeightNotAccessible);
                }
                slot_number
            }
        };
        // If the caller provided a true slot number, find the associated rollup height.
        let rollup_height = match height {
            StateToAccess::RollupHeight(rollup_height) | StateToAccess::TrueSlotNumber(_, Some(rollup_height))=> rollup_height,
            StateToAccess::TrueSlotNumber(slot_number, None) => {
                kernel
                .true_slot_number_to_rollup_height(slot_number, &mut state)
                .unwrap_or_else(|| panic!("Visible slot number not available for slot_number {slot_number}, but that slot exists in storage. This is a bug. Please report it."))
            }
        };
        // Use the slot number to find the visible slot number.
        let Some(visible_slot_number) = kernel.visible_slot_number_at(true_slot_number, &mut state)
        else {
            panic!("Visible slot number not available at slot number {true_slot_number}, but that height exist in storage. This is a bug. Please report it.");
        };
        // Use the rollup height to find the base fee per gas.
        let Some(base_fee_per_gas) = kernel.base_fee_per_gas_at(rollup_height, &mut state) else {
            panic!("Base fee per gas not available at height {rollup_height}, but that height exist in storage. This is a bug. Please report it.");
        };
        state.visible_slot_number = Some(visible_slot_number);
        state.safe_true_slot_number_to_use = Some(true_slot_number);
        state.set_gas_price(base_fee_per_gas);

        // Clear out any new values that were put in cache during initialization. Otherwise, we'd incorrectly estimate gas costs for
        // the first accesses to those values since they would be incorrectly shown as cached.
        state.clear_caches();
        Ok(state)
    }

    /// Returns a new accessor which accesses the rollup at the specified `height`.
    /// The gas price contained in the accessor is set to the base fee per gas at the specified height.
    pub fn get_archival_state(
        &self,
        height: RollupHeight,
    ) -> Result<ApiStateAccessor<S>, ApiStateAccessorError> {
        Self::build_archival_state(
            self.storage.clone(),
            self.kernel.clone(),
            StateToAccess::RollupHeight(height),
        )
    }

    /// Get the true slot number being used for queries.
    #[cfg(feature = "test-utils")]
    pub fn true_slot_number_to_use(&self) -> SlotNumber {
        self.safe_true_slot_number_to_use.unwrap()
    }
}

impl<S: Spec> VersionReader for ApiStateAccessor<S> {
    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.visible_slot_number
            .expect("Visible slot number must be set during accessor initialization")
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.current_visible_slot_number().as_true()
    }

    fn rollup_height_to_access(&self) -> RollupHeight {
        match self.state_to_access {
            StateToAccess::RollupHeight(rollup_height) => rollup_height,
            StateToAccess::TrueSlotNumber(_slot_number, Some(rollup_height)) => rollup_height,
            StateToAccess::TrueSlotNumber(slot_number, None) => {
                panic!("Rollup height not cached for slot number {slot_number}, but that slot exists in storage. This is a bug in ApiStateAccessor initialization. Please report it.");
            }
        }
    }
}
