use jsonrpsee::core::RpcResult;
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::macros::{expose_rpc, rpc_gen, DefaultRuntime};
use sov_modules_api::{
    Address, CallResponse, Context, DispatchCall, EncodeCall, Error, Genesis, MessageCodec, Module,
    ModuleInfo,
};
use sov_state::{StateValue, WorkingSet, ZkStorage};

pub trait TestSpec {
    type Data: Data;
}

pub trait Data:
    Clone
    + Eq
    + PartialEq
    + std::fmt::Debug
    + serde::Serialize
    + serde::de::DeserializeOwned
    + borsh::BorshSerialize
    + borsh::BorshDeserialize
    + 'static
{
}

impl Data for u32 {}

pub mod my_module {
    use super::*;

    #[derive(ModuleInfo)]
    pub struct QueryModule<C: Context, D: Data> {
        #[address]
        pub address: C::Address,

        #[state]
        pub data: StateValue<D>,
    }

    impl<C: Context, D> Module for QueryModule<C, D>
    where
        D: Data,
    {
        type Context = C;
        type Config = D;
        type CallMessage = D;

        fn genesis(
            &self,
            config: &Self::Config,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            self.data.set(config, working_set);
            Ok(())
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            _context: &Self::Context,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<CallResponse, Error> {
            self.data.set(&msg, working_set);
            Ok(CallResponse::default())
        }
    }

    pub mod query {

        use super::*;
        use crate::my_module::QueryModule;

        #[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
        pub struct QueryResponse {
            pub value: Option<String>,
        }

        #[rpc_gen(client, server, namespace = "queryModule")]
        impl<C, D: Data> QueryModule<C, D>
        where
            C: Context,
        {
            #[rpc_method(name = "queryValue")]
            pub fn query_value(
                &self,
                working_set: &mut WorkingSet<C::Storage>,
            ) -> RpcResult<QueryResponse> {
                let value = self.data.get(working_set).map(|d| format!("{:?}", d));
                Ok(QueryResponse { value })
            }
        }
    }
}

use my_module::query::{QueryModuleRpcImpl, QueryModuleRpcServer};

#[expose_rpc]
#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context, S: TestSpec> {
    pub first: my_module::QueryModule<C, S::Data>,
}

struct ActualSpec;

impl TestSpec for ActualSpec {
    type Data = u32;
}

fn main() {
    type C = ZkDefaultContext;
    type RT = Runtime<C, ActualSpec>;
    let storage = ZkStorage::new([1u8; 32]);
    let working_set = &mut WorkingSet::new(storage);
    let runtime = &mut Runtime::<C, ActualSpec>::default();
    let config = GenesisConfig::new(22);
    runtime.genesis(&config, working_set).unwrap();

    let message: u32 = 33;
    let serialized_message =
        <RT as EncodeCall<my_module::QueryModule<C, u32>>>::encode_call(message);
    let module = RT::decode_call(&serialized_message).unwrap();
    let context = C::new(Address::try_from([11; 32].as_ref()).unwrap());

    let _ = runtime
        .dispatch_call(module, working_set, &context)
        .unwrap();
}
