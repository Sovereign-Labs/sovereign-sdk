use super::ValueSetter;
use crate::{call, query};

use sov_modules_api::{
    mocks::{MockContext, MockPublicKey, ZkMockContext},
    Address, Context, PublicKey,
};
use sov_modules_api::{Module, ModuleInfo};
use sov_state::{ProverStorage, WorkingSet, ZkStorage};
use sovereign_sdk::stf::Event;

#[test]
fn test_value_setter() {
    let sender = MockPublicKey::try_from("value_setter_admin")
        .unwrap()
        .to_address();
    let working_set = WorkingSet::new(ProverStorage::temporary());

    // Test Native-Context
    {
        let context = MockContext::new(sender);
        test_value_setter_helper(context, working_set.clone());
    }
    let (_, witness) = working_set.freeze();

    // Test Zk-Context
    {
        let zk_context = ZkMockContext::new(sender);
        let zk_working_set = WorkingSet::with_witness(ZkStorage::new([0u8; 32]), witness);
        test_value_setter_helper(zk_context, zk_working_set);
    }
}

fn test_value_setter_helper<C: Context>(context: C, mut working_set: WorkingSet<C::Storage>) {
    let mut module = ValueSetter::<C>::new();
    module.genesis(&mut working_set).unwrap();

    let new_value = 99;
    let call_msg = call::CallMessage::DoSetValue(call::SetValue { new_value });

    // Test events
    {
        let call_response = module.call(call_msg, &context, &mut working_set).unwrap();
        let event = &call_response.events[0];
        assert_eq!(event, &Event::new("set", "value_set: 99"));
    }

    let query_msg = query::QueryMessage::GetValue;
    let query = module.query(query_msg, &mut working_set);

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
    let sender = Address::new([9; 32]);
    let backing_store = ProverStorage::temporary();
    let native_working_set = WorkingSet::new(backing_store);

    // Test Native-Context
    {
        let context = MockContext::new(sender);
        test_err_on_sender_is_not_admin_helper(context, native_working_set.clone());
    }
    let (_, witness) = native_working_set.freeze();

    // Test Zk-Context
    {
        let zk_backing_store = ZkStorage::new([0u8; 32]);
        let zk_context = ZkMockContext::new(sender);
        let zk_working_set = WorkingSet::with_witness(zk_backing_store, witness);
        test_err_on_sender_is_not_admin_helper(zk_context, zk_working_set);
    }
}

fn test_err_on_sender_is_not_admin_helper<C: Context>(
    context: C,
    mut working_set: WorkingSet<C::Storage>,
) {
    let mut module = ValueSetter::<C>::new();
    module.genesis(&mut working_set).unwrap();
    let resp = module.set_value(11, &context, &mut working_set);

    assert!(resp.is_err());
}
