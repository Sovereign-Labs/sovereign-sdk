use crate::generators::bank::BankMessageGenerator;
use crate::impl_harness_module;

impl_harness_module!(BankHarness <= generator: BankMessageGenerator<S>);
