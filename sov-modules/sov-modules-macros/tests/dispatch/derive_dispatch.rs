mod modules;

use modules::{first_test_module, second_test_module};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis};
use sovereign_db::state_db::StateDB;

#[derive(Genesis, DispatchCall, DispatchQuery)]
struct Runtime<C: Context> {
    first: first_test_module::FirstTestStruct<C>,
    second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::{DispatchCall, DispatchQuery};
    type C = MockContext;

    let db = StateDB::temporary();
    let storage = Runtime::<C>::genesis(db).unwrap();

    let context = MockContext {
        sender: MockPublicKey::new(vec![]),
    };

    let value = 11;
    {
        let message = RuntimeCall::<C>::first(value);
        let _ = message.dispatch_call(storage.clone(), &context).unwrap();
    }

    {
        let message = RuntimeQuery::<C>::first(());
        let response = message.dispatch_query(storage.clone());
        assert_eq!(response.response, vec![value]);
    }

    let value = 22;
    {
        let message = RuntimeCall::<C>::second(value);
        let _ = message.dispatch_call(storage.clone(), &context).unwrap();
    }

    {
        let message = RuntimeQuery::<C>::second(second_test_module::TestType {});
        let response = message.dispatch_query(storage.clone());
        assert_eq!(response.response, vec![value]);
    }
}
