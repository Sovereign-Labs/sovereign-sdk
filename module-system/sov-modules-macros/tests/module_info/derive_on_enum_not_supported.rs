use sov_modules_api::{ModuleInfo, StateMap};

#[derive(ModuleInfo)]
enum TestStruct<C: sov_modules_api::Context> {
    #[state]
    TestState1(StateMap<String, String>),

    #[state]
    TestState2(C),
}

fn main() {}
