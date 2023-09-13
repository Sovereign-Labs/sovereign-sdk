use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Module, WorkingSet};
use sov_state::ProverStorage;
use sov_vec_setter::{CallMessage, VecSetter, VecSetterConfig};

// rustfmt doesn't like long lines, but it's easier to read in this case.
#[rustfmt::skip]
fn test_cases() -> Vec<(Address, CallMessage, Option<Vec<u32>>)> {
    let admin = Address::from([1; 32]);
    let not_admin = Address::from([2; 32]);

    // (sender, call, expected vec contents or None if call should fail)
    vec![
        (admin, CallMessage::PushValue(1), Some(vec![1])),
        (admin, CallMessage::PushValue(2), Some(vec![1, 2])),
        (admin, CallMessage::PopValue, Some(vec![1])),
        (not_admin, CallMessage::PopValue, None),
        (admin, CallMessage::PopValue, Some(vec![])),
        (not_admin, CallMessage::SetValue { index: 0, value: 10 }, None),
        (admin, CallMessage::SetValue { index: 0, value: 10 }, None),
        (admin, CallMessage::PushValue(8), Some(vec![8])),
        (admin, CallMessage::SetValue { index: 0, value: 10 }, Some(vec![10])),
        (admin, CallMessage::PushValue(0), Some(vec![10, 0])),
        (admin, CallMessage::SetAllValues(vec![11, 12]), Some(vec![11, 12])),
        (not_admin, CallMessage::SetAllValues(vec![]), None),
    ]
}

#[test]
#[cfg(feature = "native")]
fn test_vec_setter_calls() {
    let tmpdir = tempfile::tempdir().unwrap();

    let storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage);

    let admin = Address::from([1; 32]);
    let config = VecSetterConfig { admin };

    let vec_setter = VecSetter::default();
    vec_setter.genesis(&config, &mut working_set).unwrap();

    for (sender, call, expected_contents) in test_cases().iter().cloned() {
        let context = DefaultContext::new(sender);

        let call_result = vec_setter.call(call, &context, &mut working_set);

        if call_result.is_ok() {
            let vec_contents = vec_setter.vector.iter(&mut working_set).collect::<Vec<_>>();
            assert_eq!(Some(vec_contents), expected_contents);
        } else {
            assert_eq!(expected_contents, None);
        }
    }
}
