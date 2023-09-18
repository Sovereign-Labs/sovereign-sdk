use sov_modules_api::{Context, ModuleInfo, StateMap};

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    #[address]
    address_1: C::Address,

    #[address]
    address_2: C::Address,

    #[state]
    test_state1: StateMap<u32, u32>,

    #[state]
    test_state2: StateMap<Vec<u8>, u64>,
}

fn main() {}
