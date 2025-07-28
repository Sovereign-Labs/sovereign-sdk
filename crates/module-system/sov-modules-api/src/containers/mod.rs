mod versioned_value;

pub(crate) mod map;
pub(crate) mod value;
pub(crate) mod vec;

use std::fmt::Display;
use std::str::FromStr;

use map::NamespacedStateMap;
pub use map::{AccessoryStateMap, KernelStateMap, StateMap, StateMapError};
use nearly_linear::{DropGuard, DropWarning};
use sov_state::{CompileTimeNamespace, SlotKey, StateCodec, StateItemCodec};
use value::NamespacedStateValue;
pub use value::{AccessoryStateValue, KernelStateValue, StateValue, StateValueError};
pub use vec::{AccessoryStateVec, KernelStateVec, StateVec};
pub use versioned_value::VersionedStateValue;

use crate::StateWriter;

/// A borrowed state value which points to state variable that it came from
// We use this struct to extend the borrow checker to state variables as if `self.state_value` was a reference.
// rather than a clone of a value from the external `impl TxState` struct. This borrow is purely imaginary.
// For now, all of the data is still actually cloned. In a future iteration, we might stored an `Arc` to the original value.
// in state which would allow us to avoid cloning/deserializing on each "borrow".
pub struct Borrowed<'a, T, U> {
    value: T,
    _reference: &'a U,
}

impl<T: std::fmt::Debug, U> std::fmt::Debug for Borrowed<'_, T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.value)
    }
}

impl<T: std::fmt::Display, U> std::fmt::Display for Borrowed<'_, T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl<T, U> std::ops::Deref for Borrowed<'_, T, U> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T, U> Borrowed<'a, T, U> {
    /// Builds a borrowed state value
    pub(crate) fn new(value: T, _reference: &'a U) -> Self {
        Self { value, _reference }
    }
}

impl<'a, T, U> Borrowed<'a, Option<T>, U> {
    /// Unwraps the `Option`, panicking if the value is `None`.
    pub fn unwrap(self) -> Borrowed<'a, T, U> {
        Borrowed::new(self.value.unwrap(), self._reference)
    }

    /// Unwraps the `Option`, panicking with the provided message if the value is `None`.
    pub fn expect(self, msg: &str) -> Borrowed<'a, T, U> {
        Borrowed::new(self.value.expect(msg), self._reference)
    }

    /// Unwraps the `Option`, returning the provided default value if the value is `None`.
    pub fn unwrap_or(self, default: T) -> Borrowed<'a, T, U> {
        Borrowed::new(self.value.unwrap_or(default), self._reference)
    }

    /// Unwraps the `Option`, returning an error if the value is `None`.
    pub fn ok_or_else<E>(self, default: impl FnOnce() -> E) -> Result<Borrowed<'a, T, U>, E> {
        Ok(Borrowed::new(
            self.value.ok_or_else(default)?,
            self._reference,
        ))
    }

    /// Unwraps the `Option`, returning an error if the value is `None`.
    pub fn ok_or<E>(self, err: E) -> Result<Borrowed<'a, T, U>, E> {
        Ok(Borrowed::new(self.value.ok_or(err)?, self._reference))
    }
}

/// A mutable borrowed state value which points to state variable that it came from
pub struct BorrowedMut<'a, T, U> {
    key: SlotKey,
    value: DropGuard<T>,
    reference: &'a mut U,
}

// TODO: Add PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Etc
impl<T: std::fmt::Debug, U> std::fmt::Debug for BorrowedMut<'_, T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::ops::Deref;
        write!(f, "{:?}", self.value.deref())
    }
}

impl<T: std::fmt::Display, U> std::fmt::Display for BorrowedMut<'_, T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::ops::Deref;
        write!(f, "{}", self.value.deref())
    }
}

impl<T, U> std::ops::Deref for BorrowedMut<'_, T, U> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, U> std::ops::DerefMut for BorrowedMut<'_, T, U> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<'a, T, U> BorrowedMut<'a, T, U> {
    /// Builds a borrowed state value
    pub(crate) fn new(key: SlotKey, value: T, reference: &'a mut U) -> Self {
        Self {
            key,
            value: DropGuard::new(value),
            reference,
        }
    }

    /// Discards the mutably borrowed value without saving it. This reverts any changes made while it was borrowed.
    pub fn discard(self) {
        self.value.done();
    }
}

impl<'a, T, U> BorrowedMut<'a, Option<T>, U> {
    /// Unwraps the `Option`, panicking if the value is `None`.
    pub fn unwrap(self) -> BorrowedMut<'a, T, U> {
        BorrowedMut::new(self.key, self.value.done().unwrap(), self.reference)
    }

    /// Unwraps the `Option`, panicking with the provided message if the value is `None`.
    pub fn expect(self, msg: &str) -> BorrowedMut<'a, T, U> {
        BorrowedMut::new(self.key, self.value.done().expect(msg), self.reference)
    }

    /// Unwraps the `Option`, returning the provided default value if the value is `None`.
    pub fn unwrap_or(self, default: T) -> BorrowedMut<'a, T, U> {
        BorrowedMut::new(
            self.key,
            self.value.done().unwrap_or(default),
            self.reference,
        )
    }

    /// Unwraps the `Option`, returning an error if the value is `None`.
    pub fn ok_or_else<E>(self, default: impl FnOnce() -> E) -> Result<BorrowedMut<'a, T, U>, E> {
        Ok(BorrowedMut::new(
            self.key,
            self.value.done().ok_or_else(default)?,
            self.reference,
        ))
    }

    /// Unwraps the `Option`, returning an error if the value is `None`.
    pub fn ok_or<E>(self, err: E) -> Result<BorrowedMut<'a, T, U>, E> {
        Ok(BorrowedMut::new(
            self.key,
            self.value.done().ok_or(err)?,
            self.reference,
        ))
    }
}

impl<N, V, Codec> BorrowedMut<'_, V, NamespacedStateValue<N, V, Codec>>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    N: CompileTimeNamespace,
{
    /// Saves a mutably borrowed value back to the map.
    pub fn save<Writer: StateWriter<N>>(
        self,
        state: &mut Writer,
    ) -> Result<(), <Writer as StateWriter<N>>::Error> {
        let value = self.reference.slot_value(&self.value.done());
        state.set(&self.key, value)
    }

    /// Deletes the mutably borrowed value from the map.
    pub fn delete<Writer: StateWriter<N>>(
        self,
        state: &mut Writer,
    ) -> Result<(), <Writer as StateWriter<N>>::Error> {
        self.value.done();
        state.delete(&self.key)
    }
}

impl<N, K, V, Codec> BorrowedMut<'_, V, NamespacedStateMap<N, K, V, Codec>>
where
    K: FromStr + Display,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<K>,
    N: CompileTimeNamespace,
{
    /// Saves a mutably borrowed value back to the map.
    pub fn save<Writer: StateWriter<N>>(
        self,
        state: &mut Writer,
    ) -> Result<(), <Writer as StateWriter<N>>::Error> {
        let value = self.reference.slot_value(&self.value.done());
        let key = self.key.clone();
        state.set(&key, value)
    }

    /// Deletes the mutably borrowed value from the map.
    pub fn delete<Writer: StateWriter<N>>(
        self,
        state: &mut Writer,
    ) -> Result<(), <Writer as StateWriter<N>>::Error> {
        self.value.done();
        state.delete(&self.key)
    }
}

#[cfg(test)]
mod test {
    use sov_mock_da::MockDaSpec;
    use sov_mock_zkvm::MockZkvm;
    use sov_rollup_interface::common::{IntoSlotNumber, SlotNumber};
    use sov_state::namespaces::User;
    use sov_state::{
        DefaultStorageSpec, NativeStorage, ProverStorage, SlotKey, SlotValue, Storage,
    };
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::validate_and_materialize;

    use crate::capabilities::mocks::MockKernel;
    use crate::execution_mode::Native;
    use crate::{CryptoSpec, StateWriter, WorkingSet};

    type StorageSpec = DefaultStorageSpec<TestHasher>;
    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
    type TestHasher = <<TestSpec as crate::Spec>::CryptoSpec as CryptoSpec>::Hasher;

    #[derive(Clone)]
    struct TestCase {
        key: SlotKey,
        value: SlotValue,
        version: SlotNumber,
    }

    fn create_tests() -> Vec<TestCase> {
        vec![
            TestCase {
                key: SlotKey::from_slice(b"key_0"),
                value: SlotValue::from("value_0"),
                version: 0.to_slot_number(),
            },
            TestCase {
                key: SlotKey::from_slice(b"key_1"),
                value: SlotValue::from("value_1"),
                version: 1.to_slot_number(),
            },
            TestCase {
                key: SlotKey::from_slice(b"key_2"),
                value: SlotValue::from("value_2"),
                version: 2.to_slot_number(),
            },
            TestCase {
                key: SlotKey::from_slice(b"key_1"),
                value: SlotValue::from("value_3"),
                version: 3.to_slot_number(),
            },
        ]
    }

    #[test]
    fn test_jmt_storage() -> anyhow::Result<()> {
        let mut storage_manager = SimpleStorageManager::<StorageSpec>::new();
        let mut prev_root = <ProverStorage<StorageSpec> as Storage>::PRE_GENESIS_ROOT;
        let tests = create_tests();
        {
            let mut kernel = MockKernel::<TestSpec>::default();
            for test in &tests {
                {
                    let storage = storage_manager.create_storage();
                    let mut working_set: WorkingSet<TestSpec> =
                        WorkingSet::new_with_kernel(storage.clone(), &kernel);
                    StateWriter::<User>::set(&mut working_set, &test.key, test.value.clone())?;
                    let (scratchpad, _gas_meter, _) = working_set.finalize();
                    let checkpoint = scratchpad.commit();
                    let (cache, _, witness) = checkpoint.freeze();
                    let (new_root, change_set) =
                        validate_and_materialize(storage, cache, &witness, prev_root)
                            .expect("storage is valid");
                    prev_root = new_root;
                    storage_manager.commit(change_set);
                    kernel.increase_heights();
                    let storage = storage_manager.create_storage();
                    assert_eq!(
                        Some(test.value.clone()),
                        storage.get_historical::<User>(&test.key, None, &witness),
                        "Prover storage does not have correct value"
                    );
                }
            }
        }

        {
            let storage = storage_manager.create_storage();
            for test in tests {
                assert_eq!(
                    Some(test.value),
                    storage.get_historical::<User>(
                        &test.key,
                        Some(test.version),
                        &Default::default()
                    )
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_restart_lifecycle() -> anyhow::Result<()> {
        let mut storage_manager = SimpleStorageManager::new();
        {
            let storage = storage_manager.create_storage();
            assert!(storage.is_empty());
        }

        let key = SlotKey::from_slice(b"some_key");
        let value = SlotValue::from("some_value");
        // First restart
        {
            let storage = storage_manager.create_storage();
            assert!(storage.is_empty());
            let mut working_set: WorkingSet<TestSpec> =
                WorkingSet::new_with_kernel(storage.clone(), &MockKernel::<TestSpec>::default());
            StateWriter::<User>::set(&mut working_set, &key, value.clone())?;
            let (cache, _, witness) = working_set.finalize().0.commit().freeze();
            let (_, change_set) = validate_and_materialize(
                storage,
                cache,
                &witness,
                <ProverStorage<StorageSpec> as Storage>::PRE_GENESIS_ROOT,
            )
            .expect("storage is valid");
            storage_manager.commit(change_set);
        }

        // Correctly restart from disk
        {
            let storage = storage_manager.create_storage();
            assert_eq!(
                Some(value),
                storage.get_historical::<User>(&key, None, &Default::default())
            );
        }

        Ok(())
    }
}
