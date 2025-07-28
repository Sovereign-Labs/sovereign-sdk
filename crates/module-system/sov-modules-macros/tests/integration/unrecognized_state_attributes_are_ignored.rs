use sov_modules_api::{ModuleId, ModuleInfo, Spec, StateMap};

#[derive(Clone, ModuleInfo)]
struct TestStruct<S: Spec> {
    #[id]
    id: ModuleId,

    // Unsupported attributes should be ignored to guarantee compatibility with
    // other macros.
    #[allow(dead_code)]
    #[state]
    test_state1: StateMap<u32, S::Address>,
}
