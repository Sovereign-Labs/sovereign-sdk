//! Regression test for <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/163>.

use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{ModuleId, Spec};

#[derive(sov_modules_api::ModuleInfo, Clone)]
pub struct TestStruct<S: Spec> {
    #[id]
    pub(crate) id: ModuleId,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

#[rpc_gen(client, server, namespace = "test")]
impl<S: Spec> TestStruct<S> {
    #[rpc_method(name = "foo")]
    pub fn foo(&self) -> jsonrpsee::core::RpcResult<u32> {
        Ok(42)
    }
}
