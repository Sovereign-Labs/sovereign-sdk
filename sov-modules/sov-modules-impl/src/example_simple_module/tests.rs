use crate::example_simple_module::call::{CallMessage, SetValue};
use crate::example_simple_module::query::QueryMessage;

use super::ValueAdderModule;
use sov_modules_api::mocks::MockContext;
use sov_modules_api::{Module, ModuleInfo};
use sov_state::JmtStorage;

#[test]
fn test_simple_module() {
    type C = MockContext;
    let storage = JmtStorage::default();
    let mut module = ValueAdderModule::<C>::new(storage);

    module.genesis();

    let context = MockContext { sender: todo!() };

    let call_msg = CallMessage::DoSetValue(SetValue { new_value: 99 });

    let _ = module.call(call_msg, context);

    let query_msg = QueryMessage::GetValue;
    let query = module.query(query_msg);
}
