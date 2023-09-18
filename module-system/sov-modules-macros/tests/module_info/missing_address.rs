use sov_modules_api::{ModuleInfo, StateMap};

#[derive(ModuleInfo)]
struct TestStruct<C: sov_modules_api::Context> {
    #[state]
    test_state1: StateMap<u32, u32>,

    #[state]
    test_state2: StateMap<Vec<u8>, u64>,

    #[state]
    c: C,
}

fn main() {}
