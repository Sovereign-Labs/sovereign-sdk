use super::ValueSetter;
use crate::{call, query};

use sov_modules_api::mocks::ZkMockContext;
use sov_modules_api::mocks::{MockContext, MockPublicKey};
use sov_modules_api::Context;
use sov_modules_api::{Module, ModuleInfo};
use sov_state::{JmtStorage, ZkStorage};
use sovereign_sdk::stf::Event;

#[test]
fn test_value_setter() {
    let sender = MockPublicKey::try_from("admin").unwrap();
    let storage = JmtStorage::temporary();

    let context = MockContext::new(sender.clone(), storage);
    let storage = context.make_storage();

    // Test Native-Context
    {
        test_value_setter_helper(context);
    }

    // Test Zk-Context
    {
        let zk_context = ZkMockContext::new(sender, ZkStorage::new(storage.get_first_reads()));

        test_value_setter_helper(zk_context);
    }
}

fn test_value_setter_helper<C: Context>(context: C) {
    let mut module = ValueSetter::<C>::new(context.make_storage());
    module.genesis().unwrap();

    let new_value = 99;
    let call_msg = call::CallMessage::DoSetValue(call::SetValue { new_value });

    // Test events
    {
        let call_response = module.call(call_msg, &context).unwrap();
        let event = &call_response.events[0];
        assert_eq!(event, &Event::new("set", "value_set: 99"));
    }

    let query_msg = query::QueryMessage::GetValue;
    let query = module.query(query_msg);

    // Test query
    {
        let query_response: Result<query::Response, _> = serde_json::from_slice(&query.response);

        assert_eq!(
            query::Response {
                value: Some(new_value)
            },
            query_response.unwrap()
        )
    }
}

#[test]
fn test_err_on_sender_is_not_admin() {
    let sender = MockPublicKey::try_from("not_admin").unwrap();
    let storage = JmtStorage::temporary();
    let context = MockContext::new(sender.clone(), storage.clone());
    // Test Native-Context
    {
        test_err_on_sender_is_not_admin_helper(context);
    }

    // Test Zk-Context
    {
        let zk_context = ZkMockContext::new(sender, ZkStorage::new(storage.get_first_reads()));
        test_err_on_sender_is_not_admin_helper(zk_context);
    }
}

fn test_err_on_sender_is_not_admin_helper<C: Context>(context: C) {
    let mut module = ValueSetter::<C>::new(context.make_storage());
    module.genesis().unwrap();
    let resp = module.set_value(11, &context);

    assert!(resp.is_err());
}
