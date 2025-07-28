use sov_modules_api::{ModuleInfo, StateMap};

#[derive(Clone, ModuleInfo)]
struct TestStruct<S: sov_modules_api::Spec> {
    #[state]
    test_state1: StateMap<u32, u32>,

    #[state]
    test_state2: StateMap<Vec<u8>, u64>,

    #[state]
    c: S,
}

fn main() {}
