mod modules;
use modules::{first_test_module, second_test_module,
              third_test_module,
              fourth_test_module, fourth_test_module::MyStruct, fourth_test_module::MyNewStruct, fourth_test_module::NestedEnum};
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
    let event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant1(10));
    assert_eq!(event.get_key_string(), "first-Variant1");
    let event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant2);
    assert_eq!(event.get_key_string(), "first-Variant2");
    let event = RuntimeEvent::<DefaultContext>::first(first_test_module::MyEvent::Variant3(vec![1; 3]));
    assert_eq!(event.get_key_string(), "first-Variant3");
    let event = RuntimeEvent::<DefaultContext>::second(second_test_module::MyEvent::Variant);
    assert_eq!(event.get_key_string(), "second-Variant");
    let event = RuntimeEvent::<DefaultContext>::third(());
    assert_eq!(event.get_key_string(), "third-NA-Event-Not-Defined");
    let event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant1);
    assert_eq!(event.get_key_string(), "fourth-Variant1");
    let event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant2WithStruct(MyStruct { a: 10, b: "abc".to_string()}));
    assert_eq!(event.get_key_string(), "fourth-Variant2WithStruct");
    let event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant3WithNewTypeStruct(MyNewStruct(10)));
    assert_eq!(event.get_key_string(), "fourth-Variant3WithNewTypeStruct");
    let event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant4WithUnnamedStruct { a: 10, b: "abc".to_string()});
    assert_eq!(event.get_key_string(), "fourth-Variant4WithUnnamedStruct");
    let event = RuntimeEvent::<DefaultContext>::fourth(fourth_test_module::MyEvent::Variant5WithNestedEnum(NestedEnum::Variant1));
    assert_eq!(event.get_key_string(), "fourth-Variant5WithNestedEnum");


    assert_eq!(RuntimeEvent::<DefaultContext>::get_all_key_strings(),
               vec!["first-Variant1", "first-Variant2", "first-Variant3",
                    "second-Variant", "third-NA-Event-Not-Defined",
                    "fourth-Variant1", "fourth-Variant2WithStruct",
                    "fourth-Variant3WithNewTypeStruct", "fourth-Variant4WithUnnamedStruct",
                    "fourth-Variant5WithNestedEnum"]);
}
