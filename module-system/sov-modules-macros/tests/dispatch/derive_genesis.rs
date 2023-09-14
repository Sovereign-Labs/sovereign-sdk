mod modules;

use modules::third_test_module::{self, ModuleThreeStorable};
use modules::{first_test_module, second_test_module};
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{Context, DispatchCall, Genesis, MessageCodec};
use sov_state::ZkStorage;

// Debugging hint: To expand the macro in tests run: `cargo expand --test tests`
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
    type C = ZkDefaultContext;
    let storage = ZkStorage::new();
    let mut working_set = &mut sov_modules_api::WorkingSet::new(storage);
    let runtime = &mut Runtime::<C, u32>::default();
    let config = GenesisConfig::new((), (), ());
    runtime.genesis(&config, working_set).unwrap();

    {
        let response = runtime.first.get_state_value(&mut working_set);
        assert_eq!(response, 1);
    }

    {
        let response = runtime.second.get_state_value(&mut working_set);
        assert_eq!(response, 2);
    }

    {
        let response = runtime.third.get_state_value(&mut working_set);
        assert_eq!(response, Some(0));
    }
}
