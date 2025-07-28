use crate::generators::value_setter::ValueSetterMessageGenerator;
use crate::impl_harness_module;

impl_harness_module!(ValueSetterHarness <= generator: ValueSetterMessageGenerator<S>);
