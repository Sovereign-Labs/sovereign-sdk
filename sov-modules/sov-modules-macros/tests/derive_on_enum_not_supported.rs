use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;
use sov_state::StateMap;

#[derive(ModuleInfo)]
enum TestStruct<C: Context> {
    #[state]
    TestState1(StateMap<String, String, C::Storage>),

    #[state]
    TestState2(StateMap<String, String, C::Storage>),
}

fn main() {}
