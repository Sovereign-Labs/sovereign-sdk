use sov_modules_api::macros::rpc_gen;
use sov_modules_api::ModuleId;

#[derive(sov_modules_api::ModuleInfo)]
pub struct TestStruct<S: sov_modules_api::Spec> {
    #[id]
    pub(crate) id: ModuleId,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

#[rpc_gen(client, server, namespace = "test")]
impl<S: sov_modules_api::Spec> TestStruct<S> {
    #[rpc_method(name = "foo")]
    pub fn foo(
        &self,
        _state: &mut sov_modules_api::ApiStateAccessor,
    ) -> jsonrpsee::core::RpcResult<u32> {
        Ok(42)
    }
}

fn main() {}
