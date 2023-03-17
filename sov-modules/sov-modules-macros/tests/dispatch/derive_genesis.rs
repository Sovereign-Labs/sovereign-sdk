mod modules;

use modules::{first_test_module, second_test_module};
use sov_modules_api::{mocks::MockContext, Context, Module};
use sov_modules_macros::{DispatchQuery, Genesis};
use sov_state::ProverStorage;

// Debugging hint: To expand the macro in tests run: `cargo expand --test tests`
#[derive(Genesis, DispatchQuery)]
struct Runtime<C>
where
    C: Context,
{
    first: first_test_module::FirstTestStruct<C>,
    second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    use sov_modules_api::{DispatchQuery, Genesis};

    type C = MockContext;
    let storage = ProverStorage::temporary();
    let working_set = sov_state::WorkingSet::new(storage);
    Runtime::<C>::genesis(working_set.clone()).unwrap();

    {
        let message = RuntimeQuery::<C>::first(());
        let response = message.dispatch_query(working_set.clone());
        assert_eq!(response.response, vec![1]);
    }

    {
        let message = RuntimeQuery::<C>::second(second_test_module::TestType {});
        let response = message.dispatch_query(working_set.clone());
        assert_eq!(response.response, vec![2]);
    }
}
