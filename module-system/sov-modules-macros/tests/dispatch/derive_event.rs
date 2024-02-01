mod modules;
use modules::{first_test_module, second_test_module};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{Context, DispatchCall, Event, Genesis, MessageCodec};

#[derive(Genesis, DispatchCall, Event, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context> {
    pub first: first_test_module::FirstTestStruct<C>,
    pub second: second_test_module::SecondTestStruct<C>,
}

fn main() {
    // Check to see if the runtime events are getting initialized correctly
    let _event =
        RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum1(10));
    let _event = RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum2);
    let _event =
        RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum3(vec![
            1;
            3
        ]));
    let _event =
        RuntimeEvent::<DefaultContext>::second(second_test_module::Event::SecondModuleEnum);
}
