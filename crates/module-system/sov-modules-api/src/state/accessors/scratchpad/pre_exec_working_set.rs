//! Pre-execution working set implementation.

use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{Namespace, SlotKey, SlotValue};

use super::TxScratchpad;
use super::super::{StateMetricsProvider, StateProvider, UniversalStateAccessor};
use crate::capabilities::RollupHeight;
use crate::module::Spec;
use crate::{BasicGasMeter, Gas, GasMeter, GasMeteringError, GetGasPrice, VersionReader};

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

impl<S: Spec, I: StateProvider<S>> VersionReader for PreExecWorkingSet<S, I> {
    fn rollup_height_to_access(&self) -> RollupHeight {
        self.inner.rollup_height_to_access()
    }

    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.inner.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.inner.max_allowed_slot_number_to_access()
    }
}
