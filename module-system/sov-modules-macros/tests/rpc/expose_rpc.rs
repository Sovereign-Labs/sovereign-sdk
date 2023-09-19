use jsonrpsee::core::RpcResult;
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::macros::{expose_rpc, rpc_gen};
use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo, StateValue, WorkingSet};

#[derive(ModuleInfo)]
pub struct QueryModule<C: Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub data: StateValue<u8>,
}

impl<C: Context> Module for QueryModule<C> {
    type Context = C;
    type Config = u8;
    type CallMessage = u8;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<(), Error> {
        self.data.set(config, working_set);
        Ok(())
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        _context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse, Error> {
        self.data.set(&msg, working_set);
        Ok(CallResponse::default())
    }
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryResponse {
    pub value: Option<u8>,
}

#[rpc_gen(client, server, namespace = "queryModule")]
impl<C: Context> QueryModule<C> {
    #[rpc_method(name = "queryValue")]
    pub fn query_value(
        &self,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<QueryResponse> {
        Ok(QueryResponse {
            value: self.data.get(working_set),
        })
    }
}

#[expose_rpc]
struct Runtime<C: Context> {
    pub first: QueryModule<C>,
}

fn main() {}
