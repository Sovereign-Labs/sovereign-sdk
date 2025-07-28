//! Tests multiple SDK modules in a single Rust module/scope, to make sure
//! proc-generated code doesn't conflict.

use sov_modules_api::{ModuleId, ModuleInfo, Spec, StateMap};

#[derive(Clone, ModuleInfo)]
struct Module1<SpecWithWeirdNameForTesting: Spec> {
    #[id]
    pub id: ModuleId,

    #[allow(unused)]
    #[state]
    pub balance: StateMap<SpecWithWeirdNameForTesting::Address, u32>,
}

#[derive(Clone, ModuleInfo)]
struct Module2<SpecWithWeirdNameForTesting: Spec> {
    #[id]
    pub id: ModuleId,

    #[allow(unused)]
    #[state]
    pub balance: StateMap<SpecWithWeirdNameForTesting::Address, u32>,
}
