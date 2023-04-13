mod modules;
use modules::{first_test_module, second_test_module};
use sov_modules_api::Address;
use sov_modules_api::ModuleInfo;
use sov_modules_api::{mocks::MockContext, Context, Genesis, Module};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::ProverStorage;

#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context> {
    first: first_test_module::FirstTestStruct<C>,
    second: second_test_module::SecondTestStruct<C>,
}

impl<C: Context> Runtime<C> {
    fn new() -> Self {
        Self {
            first: first_test_module::FirstTestStruct::<C>::new(),
            second: second_test_module::SecondTestStruct::<C>::new(),
        }
    }
}

fn main() {
    use sov_modules_api::{DispatchCall, DispatchQuery};
    type RT = Runtime<MockContext>;
    let runtime = &mut RT::new();

    let storage = ProverStorage::temporary();
    let working_set = &mut sov_state::WorkingSet::new(storage);
    let config = GenesisConfig::new((), ());
    runtime.genesis(&config, working_set).unwrap();
    let context = MockContext::new(Address::try_from([0; 32].as_ref()).unwrap());

    let value = 11;
    {
        let message = value;
        let serialized_message = RT::encode_first_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(runtime.module_address(&module), runtime.first.address());
        let _ = runtime
            .dispatch_call(module, working_set, &context)
            .unwrap();
    }

    {
        let serialized_message = RT::encode_first_query(());
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = runtime.dispatch_query(module, working_set);
        match response {
            RuntimeQueryResponse::First(contents) => assert_eq!(contents.response, vec![value]),
            _ => panic!("Wrong response"),
        }
    }

    let value = 22;
    {
        let message = value;
        let serialized_message = RT::encode_second_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(runtime.module_address(&module), runtime.second.address());

        let _ = runtime
            .dispatch_call(module, working_set, &context)
            .unwrap();
    }

    {
        let serialized_message = RT::encode_second_query(second_test_module::TestType {});
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = runtime.dispatch_query(module, working_set);
        match response {
            RuntimeQueryResponse::Second(contents) => assert_eq!(contents.response, vec![value]),
            _ => panic!("Wrong response"),
        }
    }
}
