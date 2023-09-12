use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::{Address, Context, Event, Module};
use sov_state::{ProverStorage, WorkingSet, ZkStorage};

use super::ValueSetter;
use crate::{call, query, ValueSetterConfig};

#[test]
fn test_value_setter() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let admin = Address::from([1; 32]);
    // Test Native-Context
    #[cfg(feature = "native")]
    {
        let config = ValueSetterConfig { admin };
        let context = DefaultContext::new(admin);
        test_value_setter_helper(context, &config, &mut working_set);
    }

    let (_, witness) = working_set.checkpoint().freeze();

    // Test Zk-Context
    {
        let config = ValueSetterConfig { admin };
        let zk_context = ZkDefaultContext::new(admin);
        let mut zk_working_set = WorkingSet::with_witness(ZkStorage::new(), witness);
        test_value_setter_helper(zk_context, &config, &mut zk_working_set);
    }
}

fn test_value_setter_helper<C: Context>(
    context: C,
    config: &ValueSetterConfig<C>,
    working_set: &mut WorkingSet<C::Storage>,
) {
    let module = ValueSetter::<C>::default();
    module.genesis(config, working_set).unwrap();

    let new_value = 99;
    let call_msg = call::CallMessage::SetValue(new_value);

    // Test events
    {
        module.call(call_msg, &context, working_set).unwrap();
        let event = &working_set.events()[0];
        assert_eq!(event, &Event::new("set", "value_set: 99"));
    }

    // Test query
    {
        let query_response = module.query_value(working_set).unwrap();

        assert_eq!(
            query::Response {
                value: Some(new_value)
            },
            query_response
        )
    }
}

#[test]
fn test_err_on_sender_is_not_admin() {
    let sender = Address::from([1; 32]);

    let tmpdir = tempfile::tempdir().unwrap();
    let backing_store = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut native_working_set = WorkingSet::new(backing_store);

    let sender_not_admin = Address::from([2; 32]);
    // Test Native-Context
    #[cfg(feature = "native")]
    {
        let config = ValueSetterConfig {
            admin: sender_not_admin,
        };
        let context = DefaultContext::new(sender);
        test_err_on_sender_is_not_admin_helper(context, &config, &mut native_working_set);
    }
    let (_, witness) = native_working_set.checkpoint().freeze();

    // Test Zk-Context
    {
        let config = ValueSetterConfig {
            admin: sender_not_admin,
        };
        let zk_backing_store = ZkStorage::new();
        let zk_context = ZkDefaultContext::new(sender);
        let zk_working_set = &mut WorkingSet::with_witness(zk_backing_store, witness);
        test_err_on_sender_is_not_admin_helper(zk_context, &config, zk_working_set);
    }
}

fn test_err_on_sender_is_not_admin_helper<C: Context>(
    context: C,
    config: &ValueSetterConfig<C>,
    working_set: &mut WorkingSet<C::Storage>,
) {
    let module = ValueSetter::<C>::default();
    module.genesis(config, working_set).unwrap();
    let resp = module.set_value(11, &context, working_set);

    assert!(resp.is_err());
}
