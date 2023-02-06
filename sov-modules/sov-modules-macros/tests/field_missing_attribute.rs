mod utils;
use sov_modules_macros::ModuleInfo;
use utils::{Context, TestState};

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    test_state1: TestState<C::Storage>,

    #[state]
    test_state2: TestState<C::Storage>,
}

fn main() {}
