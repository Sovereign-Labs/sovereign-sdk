use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::common::SlotNumber;
use sov_state::{BorshCodec, Kernel, Namespace, Prefix, StateCodec, StateItemCodec};
use unwrap_infallible::UnwrapInfallible;

use super::map::NamespacedStateMap;
use crate::{KernelStateAccessor, PrivilegedKernelAccessor, Spec, StateReader, VersionReader};

/// A `versioned` value stored in kernel state.
///
/// The semantics of this type are different
/// depending on the priveleges of the accessor. For a standard ("user space") interaction
/// via a `VersionedStateReadWriter`, only one version of this value is accessible. Inside the kernel,
/// (where access is mediated by a [`KernelStateAccessor`]), all versions of this value are accessible.
///
/// Under the hood, a versioned value is implemented as a map from a rollup
/// height to a value. From the kernel, any value can be accessed.
// TODO: Automatically clear out old versions from state https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/383
#[derive(
    Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, serde::Serialize, serde::Deserialize,
)]
pub struct VersionedStateValue<V, Codec = BorshCodec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    _phantom: std::marker::PhantomData<V>,
    elems: NamespacedStateMap<Kernel, SlotNumber, V, Codec>,
}

impl<V, Codec> VersionedStateValue<V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    /// The namespace where the versioned state value is stored.
    pub const NAMESPACE: Namespace = Namespace::Kernel;

    /// Creates a new [`VersionedStateValue`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            elems: NamespacedStateMap::with_codec(prefix, codec),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns the prefix used when this [`VersionedStateValue`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.elems.prefix
    }
}

impl<V, Codec> VersionedStateValue<V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    /// Returns the codec used by the versioned state value.
    pub fn codec(&self) -> &Codec {
        self.elems.codec()
    }

    /// Any version-aware working set can read the current contents of a versioned value.
    ///
    /// # Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the read operation.
    pub fn get_current<Reader: VersionReader + StateReader<Kernel>>(
        &self,
        state: &mut Reader,
    ) -> Result<Option<V>, <Reader as StateReader<Kernel>>::Error> {
        self.elems.get(&state.current_visible_slot_number(), state)
    }

    /// Get the current value at the latest *true* slot number.
    pub fn get_true_current<Accessor: PrivilegedKernelAccessor + StateReader<Kernel>>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<V>, <Accessor as StateReader<Kernel>>::Error> {
        self.elems.get(&state.true_slot_number(), state)
    }

    /// Only the kernel working set can write to versioned values
    ///
    /// # Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the write operation.
    pub fn set_true_current<Accessor: PrivilegedKernelAccessor>(
        &mut self,
        value: &V,
        state: &mut Accessor,
    ) -> Result<(), Accessor::Error> {
        self.elems.set(&state.true_slot_number(), value, state)
    }

    /// Only the kernel working set can write to versioned values
    ///
    /// # Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the write operation.
    pub fn set<S: Spec>(
        &mut self,
        key: &SlotNumber,
        value: &V,
        state: &mut KernelStateAccessor<'_, S>,
    ) {
        self.elems.set(key, value, state).unwrap_infallible();
    }

    /// Any version-aware working set can read the current contents of a versioned value.
    ///
    /// # Error
    /// This method can fail if the gas meter doesn't have enough funds to pay for the read operation.
    pub fn get<Reader: VersionReader + StateReader<Kernel>>(
        &self,
        key: &SlotNumber,
        state: &mut Reader,
    ) -> Result<Option<V>, Reader::Error> {
        if key.get() > state.max_allowed_slot_number_to_access().get() {
            return Ok(None);
        }
        self.elems.get(key, state)
    }
}

#[cfg(test)]
mod tests {
    use sov_mock_zkvm::MockZkvm;
    use sov_rollup_interface::common::IntoSlotNumber;
    use sov_rollup_interface::execution_mode::Native;
    use sov_state::{BorshCodec, Prefix};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::MockDaSpec;
    use unwrap_infallible::UnwrapInfallible;

    use crate::capabilities::mocks::MockKernel;
    use crate::capabilities::RollupHeight;
    use crate::runtime::capabilities::Kernel as _;
    use crate::{StateCheckpoint, VersionedStateValue};

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_kernel_state_value_as_value() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let kernel = MockKernel::<TestSpec>::new(4, 1);
        let mut state = StateCheckpoint::new(storage, &kernel);

        let prefix = Prefix::new(b"test".to_vec());
        let mut value = VersionedStateValue::<RollupHeight>::with_codec(prefix.clone(), BorshCodec);

        // Initialize a value in the kernel state during slot 4
        let mut kernel_state = kernel.accessor(&mut state);
        value
            .set_true_current(&RollupHeight::new(100), &mut kernel_state)
            .unwrap_infallible();
        assert_eq!(
            value
                .get_true_current(&mut kernel_state)
                .unwrap_infallible(),
            Some(RollupHeight::new(100))
        );

        // Try to read the value from kernel space with the rollup height set to 1. Should fail.
        assert_eq!(value.get_current(&mut state).unwrap_infallible(), None);

        // Try to read the value from kernel space with the rollup height set to 4. Should succeed.
        state.update_version(4);
        assert_eq!(
            value.get_current(&mut state).unwrap_infallible(),
            Some(RollupHeight::new(100))
        );
    }

    #[test]
    fn test_kernel_state_value_as_map() {
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();

        let kernel = MockKernel::<TestSpec>::new(4, 1);
        let mut state = StateCheckpoint::new(storage, &kernel);

        let prefix = Prefix::new(b"test".to_vec());
        let mut value = VersionedStateValue::<RollupHeight>::with_codec(prefix.clone(), BorshCodec);

        // Initialize a versioned value in the kernel state to be available starting at slot 2

        let mut kernel_state = kernel.accessor(&mut state);
        value.set(
            &2.to_slot_number(),
            &RollupHeight::new(100),
            &mut kernel_state,
        );
        assert_eq!(
            value
                .get(&2.to_slot_number(), &mut kernel_state)
                .unwrap_infallible(),
            Some(RollupHeight::new(100))
        );
        value
            .set_true_current(&RollupHeight::new(17), &mut kernel_state)
            .unwrap_infallible();

        // Try to read the value from user space with the rollup height set to 1. Should fail.
        assert_eq!(value.get_current(&mut state).unwrap_infallible(), None);

        // Try to read the value from user space with the rollup height set to 2. Should succeed.
        state.update_version(2);

        assert_eq!(
            value.get_current(&mut state).unwrap_infallible(),
            Some(RollupHeight::new(100))
        );

        // Try to read a future value from user space with the rollup height set to 1. Should fail.
        state.update_version(1);
        assert_eq!(
            value
                .get(&2.to_slot_number(), &mut state)
                .unwrap_infallible(),
            None
        );

        // Try to read the value from user space with the rollup height set to 4. Should succeed.
        state.update_version(4);
        assert_eq!(
            value.get_current(&mut state).unwrap_infallible(),
            Some(RollupHeight::new(17))
        );
    }
}
