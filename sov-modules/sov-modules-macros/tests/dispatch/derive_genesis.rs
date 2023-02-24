mod modules;

use sov_modules_api::{mocks::MockContext, Context, Module, ModuleInfo};
use sov_modules_macros::Genesis;

use modules::{first_test_module, second_test_module};

// Debugging hint: To expand the macro in tests run: `cargo expand --test tests`
#[derive(Genesis)]
struct Runtime<C>
where
    C: Context,
{
    _first: first_test_module::FirstTestStruct<C>,
    _second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::Genesis;

    type C = MockContext;
    let storage = Runtime::<C>::genesis().unwrap();

    let first_module = first_test_module::FirstTestStruct::<C>::new(storage.clone());
    let state_value = first_module.get_state_value();
    assert_eq!(state_value, Some(1));

    let second_module = second_test_module::SecondTestStruct::<C>::new(storage);
    let state_value = second_module.get_state_value();
    assert_eq!(state_value, Some(2));
}
