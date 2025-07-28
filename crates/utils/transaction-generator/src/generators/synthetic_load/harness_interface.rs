use crate::generators::synthetic_load::SyntheticLoadMessageGenerator;
use crate::impl_harness_module;

impl_harness_module!(SyntheticLoadHarness <= generator: SyntheticLoadMessageGenerator<S>);
