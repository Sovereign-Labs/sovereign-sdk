mod modules;

use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Genesis, Module, ModuleInfo,
};
use sov_modules_macros::{DispatchCall, Genesis};

use modules::{first_test_module, second_test_module};

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
    let storage = Runtime::<C>::genesis().unwrap();

    let context = MockContext {
        sender: MockPublicKey::new(vec![]),
    };

    {
        let message = RuntimeCall::<C>::_first(11);
        let _ = message.dispatch(storage.clone(), &context).unwrap();
    }

    let first_module = first_test_module::FirstTestStruct::<C>::new(storage.clone());
    let state_value = first_module.get_state_value();
    assert_eq!(state_value, Some(11));
}
