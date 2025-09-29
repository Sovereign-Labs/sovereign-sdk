use crate::generators::state_consistency::StateConsistencyMessageGenerator;
use crate::impl_harness_module;

impl_harness_module!(StateConsistencyHarness <= generator: StateConsistencyMessageGenerator<S>);