use super::AccessPatternMessageGenerator;
use crate::impl_harness_module;

impl_harness_module!(AccessPatternHarness <= generator: AccessPatternMessageGenerator<S>);
