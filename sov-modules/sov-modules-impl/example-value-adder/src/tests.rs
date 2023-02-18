use super::ValueAdderModule;
use crate::{call, query};

use sov_modules_api::mocks::ZkMockContext;
use sov_modules_api::mocks::{MockContext, MockPublicKey};
use sov_modules_api::Context;
use sov_modules_api::{Module, ModuleInfo};
use sov_state::JmtStorage;
use sov_state::ZkStorage;
use sovereign_sdk::stf::Event;

#[test]
fn test_simple_module() {
    let sender = MockPublicKey::try_from("admin").unwrap();
    let storage = JmtStorage::default();

    // Test Native-Context
    {
        let context = MockContext {
            sender: sender.clone(),
        };

        test_module(context, storage.clone());
    }

    // Test Zk-Context
    {
        let zk_context = ZkMockContext { sender };

        let zk_storage = ZkStorage::new(storage.get_first_reads());
        test_module(zk_context, zk_storage);
    }
}

fn test_module<C: Context>(context: C, storage: C::Storage) {
    let mut module = ValueAdderModule::<C>::new(storage);
    module.genesis().unwrap();

    let new_value = 99;
    let call_msg = call::CallMessage::DoSetValue(call::SetValue { new_value });

    // Test events
    {
        let call_response = module.call(call_msg, &context).unwrap();
        let event = &call_response.events[0];
        assert_eq!(event, &Event::new("add_event", "value_set: 99"));
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
