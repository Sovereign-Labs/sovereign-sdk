mod modules;

use modules::{first_test_module, second_test_module};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::JmtStorage;

#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
struct Runtime<C: Context> {
    first: first_test_module::FirstTestStruct<C>,
    second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::{DispatchCall, DispatchQuery};
    type RT = Runtime<MockContext>;

    let storage = JmtStorage::temporary();
    RT::genesis(storage.clone()).unwrap();
    let context = MockContext::new(MockPublicKey::new(vec![]));

    let value = 11;
    {
        let message = value;
        let serialized_message = RT::encode_first_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();
        let _ = module.dispatch_call(storage.clone(), &context).unwrap();
    }

    {
        let serialized_message = RT::encode_first_query(());
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = module.dispatch_query(storage.clone());
        assert_eq!(response.response, vec![value]);
    }

    let value = 22;
    {
        let message = value;
        let serialized_message = RT::encode_second_call(message);
        let module = RT::decode_call(&serialized_message).unwrap();
        let _ = module.dispatch_call(storage.clone(), &context).unwrap();
    }

    {
        let serialized_message = RT::encode_second_query(second_test_module::TestType {});
        let module = RT::decode_query(&serialized_message).unwrap();
        let response = module.dispatch_query(storage.clone());
        assert_eq!(response.response, vec![value]);
    }
}
