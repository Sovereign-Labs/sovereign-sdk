mod modules;
use modules::{first_test_module, second_test_module, third_test_module, fourth_test_module, fourth_test_module::MyStruct, fourth_test_module::MyNewStruct, fourth_test_module::NestedEnum};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{
    Context, DispatchCall, RuntimeEvent, Genesis, MessageCodec,
};

#[derive(Genesis, DispatchCall, RuntimeEvent, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct Runtime<C: Context>
{
    pub first: first_test_module::FirstTestStruct<C>,
    pub second: second_test_module::SecondTestStruct<C>,
    pub third: third_test_module::ThirdTestStruct<C, u32>,
    pub fourth: fourth_test_module::FourthTestStruct<C>,
}

fn main() {
    // Check to see if the runtime events are getting initialized correctly
    let _event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant1(10));
    let _event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant2);
    let _event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant3(vec![1; 3]));
    let _event = RuntimeEvent::<DefaultContext>::second(second_test_module::MyEvent::Variant);
    let _event = RuntimeEvent::<DefaultContext>::third(());
    let _event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant1);
    let _event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant2WithStruct(MyStruct { a: 10, b: "abc".to_string()}));
    let _event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant3WithNewTypeStruct(MyNewStruct(10)));
    let _event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant4WithUnnamedStruct { a: 10, b: "abc".to_string()});
    let _event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant5WithNestedEnum(NestedEnum::Variant1));
}
