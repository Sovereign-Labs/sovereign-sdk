mod modules;
use modules::third_test_module::{self, ModuleThreeStorable};
use modules::{first_test_module, second_test_module};
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{
    Address, Context, DispatchCall, EncodeCall, Genesis, MessageCodec, ModuleInfo,
};
use sov_state::ZkStorage;

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C, T>
where
    C: Context,
    T: ModuleThreeStorable,
{
    pub first: first_test_module::FirstTestStruct<C>,
    pub second: second_test_module::SecondTestStruct<C>,
    pub third: third_test_module::ThirdTestStruct<C, T>,
}

fn main() {
    type RT = Runtime<ZkDefaultContext, u32>;
    let runtime = &mut RT::default();

    let storage = ZkStorage::new();
    let mut working_set = &mut sov_modules_api::WorkingSet::new(storage);
    let config = GenesisConfig::new((), (), ());
    runtime.genesis(&config, working_set).unwrap();
    let context = ZkDefaultContext::new(Address::try_from([0; 32].as_ref()).unwrap());

    let value = 11;
    {
        let message = value;
        let serialized_message = <RT as EncodeCall<
            first_test_module::FirstTestStruct<ZkDefaultContext>,
        >>::encode_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(runtime.module_address(&module), runtime.first.address());
        let _ = runtime
            .dispatch_call(module, working_set, &context)
            .unwrap();
    }

    {
        let response = runtime.first.get_state_value(&mut working_set);
        assert_eq!(response, value);
    }

    let value = 22;
    {
        let message = value;
        let serialized_message = <RT as EncodeCall<
            second_test_module::SecondTestStruct<ZkDefaultContext>,
        >>::encode_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(runtime.module_address(&module), runtime.second.address());

        let _ = runtime
            .dispatch_call(module, working_set, &context)
            .unwrap();
    }

    {
        let response = runtime.second.get_state_value(&mut working_set);
        assert_eq!(response, value);
    }
}
