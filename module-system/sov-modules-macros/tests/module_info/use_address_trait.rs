//! Regression for <https://github.com/Sovereign-Labs/sovereign-sdk/issues/635>.

#![allow(unused_imports)]

use sov_modules_api::{Context, ModuleInfo, RollupAddress};

#[derive(ModuleInfo)]
struct TestModule<C: Context> {
    #[address]
    admin: C::Address,
}

fn main() {}
