//! Regression for <https://github.com/Sovereign-Labs/sovereign-sdk/issues/635>.

#![allow(unused_imports)]

use sov_modules_api::{BasicAddress, ModuleId, ModuleInfo, Spec};

#[derive(Clone, ModuleInfo)]
struct TestModule<S: Spec> {
    #[id]
    id: ModuleId,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}
