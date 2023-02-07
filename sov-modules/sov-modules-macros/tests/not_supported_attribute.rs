use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;
use sov_state::StateMap;

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    #[other]
    test_state1: StateMap<u32, String, C::Storage>,

    #[state]
    test_state2: StateMap<C::PublicKey, String, C::Storage>,
}

fn main() {}
