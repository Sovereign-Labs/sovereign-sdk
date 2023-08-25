use jsonrpsee::core::RpcResult;
pub use sov_modules_api::default_context::DefaultContext;
// use sov_modules_api::macros::{expose_rpc, rpc_gen};
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::{StateValue, WorkingSet};

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

#[derive(ModuleInfo)]
pub struct QueryModule<C: Context, D: Data> {
    #[address]
    pub address: C::Address,

    #[state]
    pub data: StateValue<D>,
}

impl<C: Context, D: Data> Module for QueryModule<C, D> {
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

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryResponse {
    pub value: Option<String>,
}

#[rpc_gen(client, server, namespace = "queryModule")]
impl<C: Context, D: Data> QueryModule<C, D> {
    #[rpc_method(name = "queryValue")]
    pub fn query_value(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<QueryResponse> {
        let value = self.data.get(working_set).map(|d| format!("{:?}", d));
        Ok(QueryResponse { value })
    }
}

struct Runtime<C: Context, D: Data> {
    pub first: QueryModule<C, D>,
}

fn main() {}
