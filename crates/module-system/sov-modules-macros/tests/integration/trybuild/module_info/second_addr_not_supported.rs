use sov_modules_api::{ModuleInfo, Spec, StateMap};

#[derive(Clone, ModuleInfo)]
struct TestStruct<S: Spec> {
    #[id]
    address_1: S::Address,

    #[id]
    address_2: S::Address,

    #[state]
    test_state1: StateMap<u32, u32>,

    #[state]
    test_state2: StateMap<Vec<u8>, u64>,
}

fn main() {}
