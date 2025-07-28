//! This test fails to compile because required traits are not implemented for the
//! generated `RuntimeEvent` enum.

use sov_modules_api::{DispatchCall, Event, Genesis, MessageCodec, Spec};

#[derive(Default, Genesis, DispatchCall, Event, MessageCodec)]
#[event(no_default_attrs)]
struct Runtime<S: Spec> {
    pub bank: sov_bank::Bank<S>,
}

fn main() {}
