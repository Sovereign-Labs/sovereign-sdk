use sov_modules_api::{ModuleId, ModuleInfo, Spec, StateMap};

#[derive(Clone, ModuleInfo)]
struct TestStruct<S: Spec> {
    #[id]
    id: ModuleId,

    #[state]
    test_state1: [usize; 22],

    #[state]
    test_state2: StateMap<u32, u32>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

fn main() {}
