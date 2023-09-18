use sov_modules_api::{Context, ModuleInfo, StateMap};

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    #[address]
    address: C::Address,

    // Unsupported attributes should be ignored to guarantee compatibility with
    // other macros.
    #[allow(dead_code)]
    #[state]
    test_state1: StateMap<u32, String>,

    #[state]
    test_state2: StateMap<C::PublicKey, String>,
}

fn main() {}
