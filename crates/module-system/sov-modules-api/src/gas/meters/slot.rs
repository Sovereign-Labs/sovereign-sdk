use sov_rollup_interface::da::DaSpec;

use crate::{GasArray, GasMeteringError, Spec, PREFERRED_DATA_FRACTION};

/// A gas meter that tracks the gas used for a slot.
pub struct SlotGasMeter<S: Spec> {
    preferred_sequencer: Option<<S::Da as DaSpec>::Address>,
    initial_slot_gas: S::Gas,
    // Assumption: The preferred batches/proofs are executed before standard batches/proofs.
    remaining_preferred_slot_gas: S::Gas,
    remaining_total_slot_gas: S::Gas,
}

impl<S: Spec> SlotGasMeter<S> {
    /// Creates a new `SlotGasMeter`
    ///
    /// # Panics
    /// May panic with an overflow if the PREFERRED_DATA_FRACTION is defined to be greater than one,
    /// which is a logic error. Will not panic under normal conditions.
    pub fn new(
        remaining_slot_gas: S::Gas,
        preferred_sequencer: Option<<S::Da as DaSpec>::Address>,
    ) -> Self {
        let mut remaining_preferred_slot_gas = remaining_slot_gas;
        remaining_preferred_slot_gas.scalar_division(PREFERRED_DATA_FRACTION.denominator.into());
        remaining_preferred_slot_gas = remaining_preferred_slot_gas
            .checked_scalar_product(PREFERRED_DATA_FRACTION.numerator.into())
            // This cannot overflow because the PREFERRED_DATA_FRACTION must be less than 1.
            .unwrap();

        Self {
            preferred_sequencer,
            remaining_preferred_slot_gas,
            initial_slot_gas: remaining_slot_gas,
            remaining_total_slot_gas: remaining_slot_gas,
        }
    }

    /// Get the remaining slot gas.
    pub fn remaining_slot_gas(&self, sequencer: &<S::Da as DaSpec>::Address) -> &S::Gas {
        if Some(sequencer) == self.preferred_sequencer.as_ref() {
            &self.remaining_preferred_slot_gas
        } else {
            &self.remaining_total_slot_gas
        }
    }

    /// Get the remaining gas allowed for the preferred sequencer.
    pub fn remaining_preferred_slot_gas(&self) -> &S::Gas {
        &self.remaining_preferred_slot_gas
    }

    /// Charge gas.
    ///
    /// # Errors
    /// Raises an error if the gas to charge is greater than the funds available
    pub fn charge_gas(
        &mut self,
        gas: &S::Gas,
        sequencer: &<S::Da as DaSpec>::Address,
    ) -> Result<(), GasMeteringError<S::Gas>> {
        // Preferred transactions reduce both the "preferred" gas and the "total" gas,
        // while standard transactions only reduce the `total` gas.
        // The `initial total` gas is always greater than the "initial preferred" gas, and preferred
        // transactions are executed before standard transactions.
        // This mechanism ensures that the preferred sequencer cannot fully censor
        // other sequencers (or emergency registrations) by exhausting all the available gas.
        if Some(sequencer) == self.preferred_sequencer.as_ref() {
            self.remaining_preferred_slot_gas = self
                .remaining_preferred_slot_gas
                .checked_sub(gas)
                .ok_or(self.gas_error(*gas, true))?;
        }

        self.remaining_total_slot_gas = self
            .remaining_total_slot_gas
            .checked_sub(gas)
            .ok_or(self.gas_error(*gas, false))?;

        Ok(())
    }

    /// Total gas used in the slot.
    pub fn total_gas_used(&self) -> S::Gas {
        self.initial_slot_gas
            .checked_sub(&self.remaining_total_slot_gas)
            .expect("The remaining_slot_gas can't be greater than the initial_slot_gas")
    }

    fn gas_error(&self, gas_to_charge: S::Gas, is_preferred: bool) -> GasMeteringError<S::Gas> {
        GasMeteringError::SlotOutOfGas {
            initial_slot_gas: self.initial_slot_gas,
            gas_to_charge,
            remaining_preferred_slot_gas: self.remaining_preferred_slot_gas,
            remaining_total_slot_gas: self.remaining_total_slot_gas,
            is_preferred,
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_mock_da::{MockAddress, MockDaSpec};
    use sov_mock_zkvm::MockZkvm;
    use sov_rollup_interface::execution_mode::Native;

    use crate::{default_spec::DefaultSpec, GasUnit};

    use super::*;

    type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    fn assert_charge_succeeds(
        meter: &mut SlotGasMeter<S>,
        gas: &GasUnit<2>,
        sequencer: &MockAddress,
    ) {
        assert!(
            meter.charge_gas(gas, sequencer).is_ok(),
            "It should be possible to charge gas"
        );
    }

    fn assert_remaining_gas_equals(
        meter: &SlotGasMeter<S>,
        sequencer: &MockAddress,
        expected: &GasUnit<2>,
    ) {
        assert_eq!(meter.remaining_slot_gas(sequencer), expected);
    }

    #[test]
    fn test_charge_gas_without_preferred_sequencer() {
        let mut slot_gas_meter = SlotGasMeter::<S>::new(GasUnit::<2>::from([100, 200]), None);
        let sequencer = MockAddress::new([10; 32]);

        let gas = GasUnit::<2>::from([10, 20]);
        assert_charge_succeeds(&mut slot_gas_meter, &gas, &sequencer);
        assert_remaining_gas_equals(&slot_gas_meter, &sequencer, &GasUnit::<2>::from([90, 180]));
    }

    #[test]
    fn test_charge_gas_from_preferred_sequencer() {
        let preferred_sequencer = MockAddress::new([10; 32]);
        let mut slot_gas_meter =
            SlotGasMeter::<S>::new(GasUnit::<2>::from([100, 200]), Some(preferred_sequencer));

        let gas = GasUnit::<2>::from([10, 20]);
        assert_charge_succeeds(&mut slot_gas_meter, &gas, &preferred_sequencer);

        let expected_preferred_gas = GasUnit::<2>::from([80, 160]);
        assert_remaining_gas_equals(
            &slot_gas_meter,
            &preferred_sequencer,
            &expected_preferred_gas,
        );
        assert_eq!(
            slot_gas_meter.remaining_preferred_slot_gas(),
            &expected_preferred_gas
        );
    }

    #[test]
    fn test_charge_gas_from_non_preferred_does_not_affect_preferred() {
        let preferred_sequencer = MockAddress::new([10; 32]);
        let mut slot_gas_meter =
            SlotGasMeter::<S>::new(GasUnit::<2>::from([100, 200]), Some(preferred_sequencer));

        // Initial preferred gas (90% of 100 = 90, 90% of 200 = 180)
        let initial_preferred_gas = *slot_gas_meter.remaining_preferred_slot_gas();

        // Charge from non-preferred sequencer
        let other_sequencer = MockAddress::new([33; 32]);
        let gas = GasUnit::<2>::from([20, 30]);
        assert_charge_succeeds(&mut slot_gas_meter, &gas, &other_sequencer);

        // Total gas should be reduced
        assert_remaining_gas_equals(
            &slot_gas_meter,
            &other_sequencer,
            &GasUnit::<2>::from([80, 170]),
        );

        // Preferred gas should remain unchanged
        assert_eq!(
            slot_gas_meter.remaining_preferred_slot_gas(),
            &initial_preferred_gas
        );
    }
}
