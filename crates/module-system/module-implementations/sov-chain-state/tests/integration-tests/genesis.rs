use sov_chain_state::ChainState;
use sov_rollup_interface::common::SlotNumber;
use sov_test_utils::AsUser;
use sov_value_setter::ValueSetter;

use crate::{setup, RT, S};

#[test]
fn chain_state_kernel_genesis() {
    let (_, runner) = setup();

    runner.query_state(|kernel| {
        assert_eq!(
            ChainState::<S>::default().true_slot_number_via_api(kernel),
            SlotNumber::GENESIS,
            "The kernel should be initialized to zero"
        );

        assert_eq!(
            0,
            ChainState::<S>::default()
                .get_next_visible_slot_number_via_api(kernel)
                .get(),
            "The kernel visible slot should be initialized to zero"
        );
    });
}

#[test]
fn test_chain_state_genesis_root() {
    let (admin, mut runner) = setup();

    let genesis_state_root = *runner.state_root();

    runner.execute(admin.create_plain_message::<RT, ValueSetter<S>>(
        sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        },
    ));

    runner.query_state(|kernel| {
        assert_eq!(
            ChainState::<S>::default().get_genesis_hash(kernel).unwrap(),
            Some(genesis_state_root),
            "The genesis hash should be set"
        );
    });
}
