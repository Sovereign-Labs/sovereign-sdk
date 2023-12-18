use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Module, StateVecAccessor, WorkingSet};
use sov_prover_storage_manager::new_orphan_storage;
use sov_vec_setter::{CallMessage, VecSetter, VecSetterConfig};

// rustfmt doesn't like long lines, but it's easier to read in this case.
#[rustfmt::skip]
fn test_cases() -> Vec<(Address, Address, CallMessage, Option<Vec<u32>>)> {
    let admin = Address::from([1; 32]);
    let not_admin = Address::from([2; 32]);
    let sequencer = Address::from([3; 32]);

    // (sender, call, expected vec contents or None if call should fail)
    vec![
        (admin, sequencer, CallMessage::PushValue(1), Some(vec![1])),
        (admin, sequencer, CallMessage::PushValue(2), Some(vec![1, 2])),
        (admin, sequencer, CallMessage::PopValue, Some(vec![1])),
        (not_admin, sequencer, CallMessage::PopValue, None),
        (admin, sequencer, CallMessage::PopValue, Some(vec![])),
        (not_admin, sequencer, CallMessage::SetValue { index: 0, value: 10 }, None),
        (admin, sequencer, CallMessage::SetValue { index: 0, value: 10 }, None),
        (admin, sequencer, CallMessage::PushValue(8), Some(vec![8])),
        (admin, sequencer, CallMessage::SetValue { index: 0, value: 10 }, Some(vec![10])),
        (admin, sequencer, CallMessage::PushValue(0), Some(vec![10, 0])),
        (admin, sequencer, CallMessage::SetAllValues(vec![11, 12]), Some(vec![11, 12])),
        (not_admin, sequencer, CallMessage::SetAllValues(vec![]), None),
    ]
}

#[test]
fn test_vec_setter_calls() {
    let tmpdir = tempfile::tempdir().unwrap();

    let storage = new_orphan_storage(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(storage);

    let admin = Address::from([1; 32]);
    let config = VecSetterConfig { admin };

    let vec_setter = VecSetter::default();
    vec_setter.genesis(&config, &mut working_set).unwrap();

    for (sender, sequencer, call, expected_contents) in test_cases().iter().cloned() {
        let context = DefaultContext::new(sender, sequencer, 1);

        let call_result = vec_setter.call(call, &context, &mut working_set);

        if call_result.is_ok() {
            let vec_contents = vec_setter.vector.iter(&mut working_set).collect::<Vec<_>>();
            assert_eq!(Some(vec_contents), expected_contents);
        } else {
            assert_eq!(expected_contents, None);
        }
    }
}
