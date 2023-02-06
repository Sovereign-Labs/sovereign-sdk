mod utils;

use sov_modules_macros::ModuleInfo;
use utils::{Context, TestState};

#[derive(ModuleInfo)]
struct TestStruct<C: Context> {
    #[state]
    test_state1: [usize; 22],

    #[state]
    test_state2: TestState<C::Storage>,
}

fn main() {}
