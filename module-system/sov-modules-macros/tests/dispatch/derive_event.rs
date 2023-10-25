mod modules;
use modules::{first_test_module, second_test_module, third_test_module};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{
    Context, DispatchCall, Event, Genesis, MessageCodec,
};

#[derive(Genesis, DispatchCall, Event, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context>
{
    pub first: first_test_module::FirstTestStruct<C>,
    pub second: second_test_module::SecondTestStruct<C>,
    pub third: third_test_module::ThirdTestStruct<C, u32>,
}

fn main() {
    // Check to see if the runtime events are getting initialized correctly
    {
        let event = RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum1(10));
        assert_eq!(event.get_key_string(), "first-FirstModuleEnum1");
    }

    {
        let event = RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum2);
        assert_eq!(event.get_key_string(), "first-FirstModuleEnum2");
    }

    {
        let event = RuntimeEvent::<DefaultContext>::first(first_test_module::Event::FirstModuleEnum3(vec![1; 3]));
        assert_eq!(event.get_key_string(), "first-FirstModuleEnum3");
    }

    {
        let event = RuntimeEvent::<DefaultContext>::second(second_test_module::Event::SecondModuleEnum);
        assert_eq!(event.get_key_string(), "second-SecondModuleEnum");
    }

    {
        // Not sure if this is how we'd want to keep this. But wanted to highlight this edge case.
        let event = RuntimeEvent::<DefaultContext>::third(());
        assert_eq!(event.get_key_string(), "third-");
    }

}
