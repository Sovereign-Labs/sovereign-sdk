use jsonrpsee::core::RpcResult;
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::macros::{expose_rpc, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::{
    decode_borsh_serialized_message, ApiStateAccessor, Context, DaSpec, DispatchCall, EncodeCall,
    Genesis, MessageCodec, Module, ModuleId, ModuleInfo, Spec, StateCheckpoint, StateValue,
    TxState,
};
use sov_state::ZkStorage;
use sov_test_utils::ZkTestSpec;

pub trait TestSpec: Default + std::fmt::Debug + Clone + PartialEq + Eq + 'static {
    type Data: Data;
}

pub trait Data:
    Clone
    + Eq
    + PartialEq
    + std::fmt::Debug
    + std::default::Default
    + serde::Serialize
    + serde::de::DeserializeOwned
    + borsh::BorshSerialize
    + borsh::BorshDeserialize
    + schemars::JsonSchema
    + UniversalWallet
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
            self.data.set(&msg, state)?;
            Ok(())
        }
    }

    pub mod rpc {
        use super::*;

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
                    .map(|d| format!("{d:?}"));
                Ok(QueryResponse { value })
            }
        }
    }
}

#[expose_rpc]
#[derive(Default, Genesis, DispatchCall, MessageCodec)]
struct Runtime<S: Spec, T: TestSpec> {
    pub first: my_module::QueryModule<S, T::Data>,
}

#[derive(Default, PartialEq, Eq, Clone, Debug)]
struct ActualSpec;

impl TestSpec for ActualSpec {
    type Data = u32;
}

#[test]
fn associated_types() {
    type S = ZkTestSpec;
    type RT = Runtime<S, ActualSpec>;
    let storage = ZkStorage::new();
    let mut state = StateCheckpoint::new(storage, &MockKernel::<S>::default());
    let runtime = &mut Runtime::<S, ActualSpec>::default();
    let config = GenesisConfig::new(22);
    let mut genesis_state = state.to_genesis_state_accessor::<RT>(&config);
    runtime
        .genesis(&Default::default(), &config, &mut genesis_state)
        .unwrap();
    let mut working_set = state.to_working_set_unmetered();

    let message: u32 = 33;
    let serialized_message =
        <RT as EncodeCall<my_module::QueryModule<S, u32>>>::encode_call(message);
    let module =
        decode_borsh_serialized_message::<<RT as DispatchCall>::Decodable>(&serialized_message)
            .unwrap();
    let sender = <S as Spec>::Address::from([11; 28]);
    let sequencer = <S as Spec>::Address::from([12; 28]);
    let sequencer_da = <<ZkTestSpec as Spec>::Da as DaSpec>::Address::new([0; 32]);
    let context = Context::<S>::new(sender, Default::default(), sequencer, sequencer_da);

    runtime
        .dispatch_call(module, &mut working_set, &context)
        .unwrap();
}
