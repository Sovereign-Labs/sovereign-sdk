mod modules;

use modules::{first_test_module, second_test_module};
use sov_modules_api::ModuleInfo;
use sov_modules_api::{mocks::MockContext, Address, Context, Genesis, Module};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::ProverStorage;

#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
struct Runtime<C: Context> {
    first: first_test_module::FirstTestStruct<C>,
    second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::{DispatchCall, DispatchQuery};
    type RT = Runtime<MockContext>;

    let storage = ProverStorage::temporary();
    let working_set = sov_state::WorkingSet::new(storage);
    RT::genesis(working_set.clone()).unwrap();
    let context = MockContext::new(Address::new([0; 32]));

    let value = 11;
    {
        let message = value;
        let serialized_message = RT::encode_first_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(
            module.module_address(),
            first_test_module::FirstTestStruct::<MockContext>::address()
        );
        let _ = module.dispatch_call(working_set.clone(), &context).unwrap();
    }

    {
        let serialized_message = RT::encode_first_query(());
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = module.dispatch_query(working_set.clone());
        assert_eq!(response.response, vec![value]);
    }

    let value = 22;
    {
        let message = value;
        let serialized_message = RT::encode_second_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();

        assert_eq!(
            module.module_address(),
            second_test_module::SecondTestStruct::<MockContext>::address()
        );

        let _ = module.dispatch_call(working_set.clone(), &context).unwrap();
    }

    {
        let serialized_message = RT::encode_second_query(second_test_module::TestType {});
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = module.dispatch_query(working_set.clone());
        assert_eq!(response.response, vec![value]);
    }
}
