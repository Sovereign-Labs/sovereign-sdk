use jsonrpsee::core::RpcResult;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::macros::{expose_rpc, rpc_gen, DefaultRuntime};
use sov_modules_api::{
    Address, CallResponse, Context, DispatchCall, EncodeCall, Error, Genesis, MessageCodec, Module,
    ModuleInfo,
};
use sov_state::{StateValue, WorkingSet, ZkStorage};

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
impl<C, D> QueryModule<C, D>
where
    C: Context,
    D: Data,
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

#[expose_rpc(DefaultContext)]
#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context, D: Data> {
    pub first: QueryModule<C, D>,
}

fn main() {
    type C = ZkDefaultContext;
    type RT = Runtime<C, u32>;
    let storage = ZkStorage::new([1u8; 32]);
    let working_set = &mut sov_state::WorkingSet::new(storage);
    let runtime = &mut Runtime::<C, u32>::default();
    let config = GenesisConfig::new(22);
    runtime.genesis(&config, working_set).unwrap();

    let message: u32 = 33;
    let serialized_message = <RT as EncodeCall<QueryModule<C, u32>>>::encode_call(message);
    let module = RT::decode_call(&serialized_message).unwrap();
    let context = C::new(Address::try_from([11; 32].as_ref()).unwrap());

    let _ = runtime
        .dispatch_call(module, working_set, &context)
        .unwrap();
}
