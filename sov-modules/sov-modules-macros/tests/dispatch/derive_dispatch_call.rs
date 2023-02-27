mod modules;

use modules::{first_test_module, second_test_module};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Genesis, Module, ModuleInfo,
};
use sov_modules_macros::{DispatchCall, Genesis};
use sovereign_db::state_db::StateDB;

#[derive(Genesis, DispatchCall)]
struct Runtime<C>
where
    C: Context,
{
    _first: first_test_module::FirstTestStruct<C>,
    _second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::DispatchCall;
    type C = MockContext;

    let db = StateDB::temporary();
    let storage = Runtime::<C>::genesis(db).unwrap();

    let context = MockContext {
        sender: MockPublicKey::new(vec![]),
    };

    let value = 11;
    {
        let message = RuntimeCall::<C>::_first(value);
        let _ = message.dispatch(storage.clone(), &context).unwrap();
    }

    let first_module = first_test_module::FirstTestStruct::<C>::new(storage.clone());
    let state_value = first_module.get_state_value();
    assert_eq!(state_value, Some(value));

    let value = 22;
    {
        let message = RuntimeCall::<C>::_second(value);
        let _ = message.dispatch(storage.clone(), &context).unwrap();
    }

    let second_module = second_test_module::SecondTestStruct::<C>::new(storage.clone());
    let state_value = second_module.get_state_value();
    assert_eq!(state_value, Some(value));
}
