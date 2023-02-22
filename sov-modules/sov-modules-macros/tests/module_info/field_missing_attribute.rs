use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;
use sov_state::StateMap;

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    test_state1: StateMap<u32, u32, C::Storage>,

    #[state]
    test_state2: StateMap<Vec<u8>, u64, C::Storage>,
}

fn main() {}
