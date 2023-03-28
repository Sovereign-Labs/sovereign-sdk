use sov_modules_macros::ModuleInfo;
use sov_state::StateMap;

#[derive(ModuleInfo)]
enum TestStruct {
    #[state]
    TestState1(StateMap<String, String>),

    #[state]
    TestState2(StateMap<String, String>),
}

fn main() {}
