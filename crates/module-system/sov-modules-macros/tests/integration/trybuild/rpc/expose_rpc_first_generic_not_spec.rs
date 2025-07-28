use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::{expose_rpc, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    ApiStateAccessor, Context, DaSpec, DispatchCall, Error, Genesis, MessageCodec, Module,
    ModuleId, ModuleInfo, Spec, StateValue, TxState,
};

pub trait TestSpec:
    Default + Send + Sync + std::fmt::Debug + Clone + PartialEq + Eq + 'static
{
    type Data: Data;
}

pub trait Data:
    Clone
    + Eq
    + PartialEq
    + std::fmt::Debug
    + Default
    + serde::Serialize
    + serde::de::DeserializeOwned
    + borsh::BorshSerialize
    + borsh::BorshDeserialize
    + schemars::JsonSchema
    + sov_modules_api::sov_universal_wallet::schema::UniversalWallet
    + Send
    + Sync
    + 'static
{
}

impl Data for u32 {}

pub mod my_module {
    use super::*;

    #[derive(ModuleInfo, Clone)]
    pub struct QueryModule<S: Spec, D: Data> {
        #[id]
        pub id: ModuleId,

        #[state]
        pub data: StateValue<D>,

        #[phantom]
        phantom: std::marker::PhantomData<S>,
    }

    impl<S: Spec, D> Module for QueryModule<S, D>
    where
        D: Data,
    {
        type Spec = S;
        type Config = D;
        type CallMessage = D;
        type Event = ();

        fn genesis(
            &mut self,
            _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

            config: &Self::Config,
            state: &mut impl sov_modules_api::GenesisState<S>,
        ) -> anyhow::Result<()> {
            self.data.set(config, state).unwrap();
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _context: &Context<Self::Spec>,
            state: &mut impl TxState<S>,
        ) -> anyhow::Result<()> {
            self.data
                .set(&msg, state)
                .map_err(|e| Error::ModuleError(e.into()))?;
            Ok(())
        }
    }

    pub mod rpc {
        use super::*;
        use crate::my_module::QueryModule;

        #[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
        pub struct QueryResponse {
            pub value: Option<String>,
        }

        #[rpc_gen(client, server, namespace = "queryModule")]
        impl<S, D: Data> QueryModule<S, D>
        where
            S: Spec,
        {
            #[rpc_method(name = "queryValue")]
            pub fn query_value(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<QueryResponse> {
                let value = self
                    .data
                    .get(state)
                    .unwrap_infallible()
                    .map(|d| format!("{:?}", d));
                Ok(QueryResponse { value })
            }
        }
    }
}

#[expose_rpc]
#[derive(Default, Genesis, DispatchCall, MessageCodec)]
struct Runtime<T: TestSpec, S: Spec> {
    pub first: my_module::QueryModule<S, T::Data>,
}

#[derive(Default, Clone, PartialEq, Eq)]
struct ActualSpec;

impl TestSpec for ActualSpec {
    type Data = u32;
}

fn main() {}
