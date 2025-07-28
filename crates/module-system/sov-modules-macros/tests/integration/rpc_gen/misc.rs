use std::hash::Hasher;

use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::{expose_rpc, rpc_gen};
use sov_modules_api::{ApiStateAccessor, DaSpec, DispatchCall, ModuleId, ModuleInfo, Spec};

#[derive(ModuleInfo, Clone)]
// Test: const generics, multiple generics, unusual `Spec` placement (not the first generic).
pub struct MyModule<S: Spec, D>
where
    // Test: `where` clause.
    D: std::hash::Hash
        + std::clone::Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    #[id]
    pub(crate) id: ModuleId,
    #[state]
    pub(crate) data: ::sov_modules_api::StateValue<D>,
    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec, D> sov_modules_api::Module for MyModule<S, D>
where
    D: std::hash::Hash
        + std::clone::Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    type Spec = S;

    type Config = ();

    type CallMessage = ();

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

        _config: &Self::Config,
        _state: &mut impl sov_modules_api::GenesisState<S>,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &sov_modules_api::Context<Self::Spec>,
        _state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
}

#[rpc_gen(client, server, namespace = "test")]
impl<S: sov_modules_api::Spec, D> MyModule<S, D>
where
    D: std::hash::Hash
        + std::clone::Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + serde::Serialize
        + serde::de::DeserializeOwned
        + std::marker::Send
        + std::marker::Sync
        + 'static,
{
    #[rpc_method(name = "a")]
    // Test: `&self`.
    pub fn a(&self) -> RpcResult<u32> {
        unimplemented!()
    }

    #[rpc_method(name = "b")]
    // Test: unused `ApiStateAccessor`.
    pub fn b(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<u32> {
        // Test: reference to `self` field.
        let _ = &self.data;
        unimplemented!()
    }

    #[rpc_method(name = "anotherMethod")]
    fn another_method(&self, result: D, _state: &mut ApiStateAccessor<S>) -> RpcResult<(D, u64)> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let value = result.clone();
        value.hash(&mut hasher);
        let hashed_value = hasher.finish();

        Ok((value, hashed_value))
    }
}

#[derive(Default, DispatchCall)]
#[expose_rpc]
pub struct TestRuntime<S: Spec> {
    module: MyModule<S, u32>,
}
