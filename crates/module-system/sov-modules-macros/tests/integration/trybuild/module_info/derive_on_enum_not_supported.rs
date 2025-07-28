use sov_modules_api::{ModuleInfo, StateMap};

#[derive(Clone, ModuleInfo)]
enum TestStruct<S: sov_modules_api::Spec> {
    #[state]
    TestState1(StateMap<String, String>),

    #[state]
    TestState2(S),
}

fn main() {}
