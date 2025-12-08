//! Pre-execution working set implementation.

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_state::{Namespace, SlotKey, SlotValue};

use super::super::{StateMetricsProvider, StateProvider, UniversalStateAccessor};
use super::TxScratchpad;
use crate::module::Spec;
use crate::state::traits::delegate_version_reader;
use crate::{BasicGasMeter, Gas, GasMeter, GasMeteringError, GetGasPrice};

/// A working set that can be used to charge gas for pre transaction execution checks.
pub struct PreExecWorkingSet<S: Spec, I: StateProvider<S>> {
    pub(super) inner: TxScratchpad<S, I>,
    pub(super) gas_meter: BasicGasMeter<S>,
}

impl<S: Spec, I: StateProvider<S>> StateMetricsProvider for PreExecWorkingSet<S, I> {
    fn metrics(&mut self) -> &mut StateMetrics {
        self.inner.metrics()
    }
}

impl<S: Spec, I: StateProvider<S>> PreExecWorkingSet<S, I> {
    /// Returns the associated gas meter and the scratchpad.
    #[must_use]
    pub fn to_scratchpad_and_gas_meter(self) -> (TxScratchpad<S, I>, BasicGasMeter<S>) {
        (self.inner, self.gas_meter)
    }

    /// Commits the contents of the [`PreExecWorkingSet`].
    #[must_use]
    pub fn commit(self) -> Self {
        let inner = self.inner.commit();
        let scratchpad = inner.to_tx_scratchpad();
        scratchpad.to_pre_exec_working_set(self.gas_meter)
    }

    /// Reverts all changes up to the last commit.
    #[must_use]
    pub fn revert(self) -> (TxScratchpad<S, I>, BasicGasMeter<S>) {
        let inner = self.inner.revert();
        let scratchpad = inner.to_tx_scratchpad();
        (scratchpad, self.gas_meter)
    }
}

impl<S: Spec, I: StateProvider<S>> GasMeter for PreExecWorkingSet<S, I> {
    type Spec = S;
    fn charge_gas(&mut self, amount: S::Gas) -> anyhow::Result<(), GasMeteringError<S::Gas>> {
        self.gas_meter.charge_gas(amount)
    }
    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        self.gas_meter.try_as_basic_gas_meter()
    }
    fn charge_linear_gas(
        &mut self,
        amount: <Self::Spec as Spec>::Gas,
        parameter: u32,
    ) -> anyhow::Result<(), GasMeteringError<<Self::Spec as Spec>::Gas>> {
        self.gas_meter.charge_linear_gas(amount, parameter)
    }

    #[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.gas_meter.remove_gas_pattern(amount, parameter);
    }
}

impl<S: Spec, I: StateProvider<S>> GetGasPrice for PreExecWorkingSet<S, I> {
    type Spec = S;
    fn gas_price(&self) -> <S::Gas as Gas>::Price {
        self.gas_meter.gas_price()
    }
}

impl<S: Spec, I: StateProvider<S>> UniversalStateAccessor for PreExecWorkingSet<S, I> {
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<u32> {
        <TxScratchpad<S, I> as UniversalStateAccessor>::get_size(
            &mut self.inner,
            namespace,
            key,
            metrics,
        )
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metrics: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        <TxScratchpad<S, I> as UniversalStateAccessor>::get_value(
            &mut self.inner,
            namespace,
            key,
            metrics,
        )
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        <TxScratchpad<S, I> as UniversalStateAccessor>::set_value(
            &mut self.inner,
            namespace,
            key,
            value,
        );
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        <TxScratchpad<S, I> as UniversalStateAccessor>::delete_value(
            &mut self.inner,
            namespace,
            key,
        );
    }
}

delegate_version_reader!(PreExecWorkingSet<S, I> where [S: Spec, I: StateProvider<S>] => inner);

#[cfg(test)]
mod tests {
    use sov_state::codec::BcsCodec;
    use sov_state::namespaces::User;
    use sov_state::SlotValueFromCodec;
    use sov_state::{SlotKey, SlotValue};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    use crate::capabilities::mocks::MockKernel;
    use crate::execution_mode::Native;
    use crate::state::accessors::StateProvider;
    use crate::{
        BasicGasMeter, GasArray, Spec, StateAccessor, StateCheckpoint, StateReader, StateWriter,
    };

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    fn save_and_check_value<ST: StateAccessor>(key: &SlotKey, val: SlotValue, accessor: &mut ST) {
        StateWriter::<User>::set(accessor, key, val.clone()).expect("This should be unfaillible");
        assert_eq!(
            Some(val),
            StateReader::<User>::get(accessor, key).expect("This should be unfaillible")
        );
    }

    #[test]
    fn test_pre_exec_ws() {
        let codec = BcsCodec {};
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        let kernel: MockKernel<TestSpec> = MockKernel::new(4, 1);

        let checkpoint = StateCheckpoint::<TestSpec>::new(storage, &kernel, None);
        let mut scratchpad = checkpoint.to_tx_scratchpad();

        // Save some values in the scratchpad.
        let storage_key_1 = SlotKey::test_key(1);
        let storage_value_1 = SlotValue::new(&vec![11], &codec);
        save_and_check_value(&storage_key_1, storage_value_1.clone(), &mut scratchpad);

        let gas_meter = BasicGasMeter::new_with_gas(
            <<TestSpec as Spec>::Gas as crate::Gas>::max(),
            <<TestSpec as Spec>::Gas as crate::Gas>::Price::ZEROED,
        );
        let mut pre_exec_ws = scratchpad.to_pre_exec_working_set(gas_meter);

        assert_eq!(
            Some(storage_value_1.clone()),
            StateReader::<User>::get(&mut pre_exec_ws, &storage_key_1)
                .expect("This should be unfaillible")
        );

        // Save some values in the pre_exec_ws
        let storage_key_2 = SlotKey::test_key(2);
        let storage_value_2 = SlotValue::new(&vec![22], &codec);
        save_and_check_value(&storage_key_2, storage_value_2.clone(), &mut pre_exec_ws);

        // Commit changes
        let mut pre_exec_ws = pre_exec_ws.commit();

        // Save some values in the pre_exec_ws
        let storage_key_3 = SlotKey::test_key(3);
        let storage_value_3 = SlotValue::new(&vec![33], &codec);
        save_and_check_value(&storage_key_3, storage_value_3.clone(), &mut pre_exec_ws);

        let (mut new_scratchpad, _) = pre_exec_ws.revert();

        // After reverting, only the values set before the `commit` should be visible.
        assert_eq!(
            Some(storage_value_1),
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_1)
                .expect("This should be unfaillible")
        );

        assert_eq!(
            Some(storage_value_2),
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_2)
                .expect("This should be unfaillible")
        );

        assert_eq!(
            None,
            StateReader::<User>::get(&mut new_scratchpad, &storage_key_3)
                .expect("This should be unfaillible")
        );
    }
}
