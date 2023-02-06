mod utils;
use sov_modules_macros::ModuleInfo;
use utils::{Context, TestState};

#[derive(ModuleInfo)]
enum TestStruct<C: Context> {
    #[state]
    TestState1(TestState<C::Storage>),

    #[state]
    TestState2(TestState<C::Storage>),
}

fn main() {}
