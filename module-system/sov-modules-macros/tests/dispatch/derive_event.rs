mod modules;
use modules::{first_test_module, second_test_module, third_test_module, fourth_test_module, fourth_test_module::MyStruct, fourth_test_module::MyNewStruct, fourth_test_module::NestedEnum};
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{
    Context, DispatchCall, Event, RuntimeEvent, Genesis, MessageCodec,
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
    assert_eq!(first_test_module::MyEvent::Variant1(10).event_key(), "Variant1");
    assert_eq!(first_test_module::MyEvent::Variant2.event_key(), "Variant2");
    assert_eq!(first_test_module::MyEvent::Variant3(vec![1;3]).event_key(), "Variant3");
    assert_eq!(first_test_module::MyEvent::Variant3(Vec::new()).event_key(), "Variant3");
    assert_eq!(second_test_module::MyEvent::Variant.event_key(), "Variant");
    assert_eq!(fourth_test_module::MyEvent::Variant1.event_key(), "Variant1");
    assert_eq!(fourth_test_module::MyEvent::Variant2WithStruct(MyStruct { a: 10, b: "abc".to_string()}).event_key(), "Variant2WithStruct");
    assert_eq!(fourth_test_module::MyEvent::Variant3WithNewTypeStruct(MyNewStruct(10)).event_key(), "Variant3WithNewTypeStruct");
    assert_eq!(fourth_test_module::MyEvent::Variant4WithUnnamedStruct { a: 10, b: "abc".to_string()}.event_key(), "Variant4WithUnnamedStruct");
    assert_eq!(fourth_test_module::MyEvent::Variant5WithNestedEnum(NestedEnum::Variant1).event_key(), "Variant5WithNestedEnum");
}
