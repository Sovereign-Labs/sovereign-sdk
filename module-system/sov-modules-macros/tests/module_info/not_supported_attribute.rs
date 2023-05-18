use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;
use sov_state::StateMap;

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    #[address]
    address: C::Address,

    #[other]
    test_state1: StateMap<u32, String>,

    #[state]
    test_state2: StateMap<C::PublicKey, String>,
}

fn main() {}
