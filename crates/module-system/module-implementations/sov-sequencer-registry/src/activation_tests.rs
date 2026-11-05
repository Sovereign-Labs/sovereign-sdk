use sov_mock_da::{MockAddress, MockBlockHeader};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::serde_json;
use sov_modules_api::{Amount, ExecutionContext, StateTransitionFunction, TxEffect, VersionReader};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{SequencerRegistry as RuntimeRegistry, TestRunner};
use sov_test_utils::storage::ForklessStorageManager;
use sov_test_utils::{
    generate_optimistic_runtime, AsUser, BatchType, TestSequencer, TestSpec, TestStfBlueprint,
    TestUser, TransactionTestCase,
};

use crate::{PendingDaAddressUpdate, SequencerRegistry};

type S = TestSpec;
generate_optimistic_runtime!(ActivationRuntime <=);
type RT = ActivationRuntime<S>;
type Message = sov_sequencer_registry::CallMessage<S>;

const BOND: Amount = Amount::new(100_000_000_000);
const HEIGHT_OVERRIDE: &str = "SOV_TEST_CONST_OVERRIDE_SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT";

fn setup(height: u64) -> (TestSequencer<S>, TestUser<S>, TestRunner<RT, S>) {
    std::env::set_var(HEIGHT_OVERRIDE, height.to_string());
    let config = HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let preferred = config.initial_sequencer.clone();
    let user = config.additional_accounts()[0].clone();
    let genesis = GenesisConfig::from_minimal_config(config.into());
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    (preferred, user, runner)
}

fn execute(runner: &mut TestRunner<RT, S>, user: &impl AsUser<S>, message: Message) -> TxEffect<S> {
    let (receipt, _) = runner.execute(user.create_plain_message::<RT, RuntimeRegistry<S>>(message));
    receipt.last_tx_receipt().receipt.clone()
}

fn register(runner: &mut TestRunner<RT, S>, user: &impl AsUser<S>, da_address: MockAddress) {
    assert!(execute(
        runner,
        user,
        Message::Register {
            da_address,
            amount: BOND
        }
    )
    .is_successful());
}

#[test]
fn registration_before_activation_is_tracked_on_first_rotation() {
    let (_, user, mut runner) = setup(3);
    let da = MockAddress::new([11; 32]);
    let rotated = MockAddress::new([12; 32]);
    let reg = SequencerRegistry::<S>::default();
    register(&mut runner, &user, da);
    runner.advance_slots(1);
    runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 2);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), None);
        assert!(reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .is_none());
    });

    assert!(execute(
        &mut runner,
        &user,
        Message::UpdateDaAddress {
            old_da_address: da,
            new_da_address: rotated
        }
    )
    .is_successful());

    runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 3);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(1));
        let current = reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .unwrap();
        assert_eq!(current.registration_id, 1);
        assert_eq!(current.current_da_address, rotated);
        assert_eq!(
            reg.known_sequencers
                .get(&rotated, state)
                .unwrap()
                .map(|known| known.address),
            Some(user.address())
        );
        assert_eq!(
            reg.retired_da_addresses
                .get(&da, state)
                .unwrap()
                .map(|retired| retired.registration_id),
            Some(1)
        );
    });
}

#[test]
fn rotation_is_rejected_before_activation_and_accepted_at_activation() {
    let (preferred, _, mut runner) = setup(2);
    let new_da = MockAddress::new([12; 32]);
    let reg = SequencerRegistry::<S>::default();
    let message = Message::UpdateDaAddress {
        old_da_address: preferred.da_address,
        new_da_address: new_da,
    };
    assert!(matches!(
        execute(&mut runner, &preferred, message.clone()),
        TxEffect::Reverted(_)
    ));
    runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 1);
        assert!(reg.pending_da_address_update.get(state).unwrap().is_none());
        assert!(reg
            .known_sequencers
            .get(&preferred.da_address, state)
            .unwrap()
            .is_some());
    });
    assert!(execute(&mut runner, &preferred, message).is_successful());
    runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 2);
        assert_eq!(
            reg.get_preferred_sequencer(state).unwrap(),
            Some((new_da, preferred.user_info.address()))
        );
        assert!(reg
            .known_sequencers
            .get(&preferred.da_address, state)
            .unwrap()
            .is_none());
        assert!(reg.pending_da_address_update.get(state).unwrap().is_none());
    });
}

#[test]
fn genesis_activation_tracks_initial_sequencer() {
    let (preferred, _, runner) = setup(0);
    runner.query_visible_state(|state| {
        let reg = SequencerRegistry::<S>::default();
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(1));
        assert_eq!(
            reg.current_da_address_by_sequencer
                .get(&preferred.user_info.address(), state)
                .unwrap()
                .unwrap()
                .registration_id,
            1
        );
    });
}

#[test]
fn exhausted_registration_ids_revert_without_locking_stake() {
    let (_, user, mut runner) = setup(0);
    let da = MockAddress::new([11; 32]);
    let owner = user.address();
    let balance = runner
        .query_visible_state(|state| TestRunner::<RT, S>::bank_gas_balance(&owner, state).unwrap());
    runner.__apply_to_state(|state| {
        SequencerRegistry::<S>::default()
            .registration_id_counter
            .set(&u64::MAX, state)
            .unwrap();
    });
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, RuntimeRegistry<S>>(Message::Register {
            da_address: da,
            amount: BOND,
        }),
        assert: Box::new(move |result, state| {
            let TxEffect::Reverted(reverted) = result.tx_receipt else {
                panic!("Exhausted registration IDs must revert");
            };
            assert!(reverted
                .reason
                .to_string()
                .contains("registration IDs exhausted"));
            let reg = SequencerRegistry::<S>::default();
            assert_eq!(
                reg.registration_id_counter.get(state).unwrap(),
                Some(u64::MAX)
            );
            assert!(reg.known_sequencers.get(&da, state).unwrap().is_none());
            assert!(reg
                .current_da_address_by_sequencer
                .get(&owner, state)
                .unwrap()
                .is_none());
            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&owner, state),
                balance.checked_sub(result.gas_value_used)
            );
        }),
    });
}

#[test]
fn registration_id_is_not_reused_after_slashing() {
    let (_, user, mut runner) = setup(0);
    let da = MockAddress::new([11; 32]);
    register(&mut runner, &user, da);
    let reg = SequencerRegistry::<S>::default();
    let previous_id = runner.query_visible_state(|state| {
        reg.current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .unwrap()
            .registration_id
    });
    runner.__apply_to_state(|state| {
        let mut reg = SequencerRegistry::<S>::default();
        reg.slash_sequencer(&da, state);
        assert_eq!(
            reg.registration_id_counter.get(state).unwrap(),
            Some(previous_id)
        );
    });
    register(&mut runner, &user, da);
    runner.query_visible_state(|state| {
        let current = reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .unwrap();
        assert_eq!(current.registration_id, previous_id + 1);
    });
}

#[test]
fn withdraw_is_refused_while_rotation_away_from_address_is_pending() {
    let (_, user, mut runner) = setup(0);
    let da = MockAddress::new([11; 32]);
    let new_da = MockAddress::new([12; 32]);
    register(&mut runner, &user, da);
    assert!(execute(
        &mut runner,
        &user,
        Message::InitiateWithdrawal { da_address: da }
    )
    .is_successful());
    runner.advance_slots(config_value!("DEFERRED_SLOTS_COUNT") + 1);
    let owner = user.address();
    runner.__apply_to_state(move |state| {
        SequencerRegistry::<S>::default()
            .pending_da_address_update
            .set(
                &PendingDaAddressUpdate {
                    sequencer: owner,
                    old_da_address: da,
                    new_da_address: new_da,
                },
                state,
            )
            .unwrap();
    });

    let TxEffect::Reverted(reverted) =
        execute(&mut runner, &user, Message::Withdraw { da_address: da })
    else {
        panic!("Withdraw must be refused while a rotation away from the address is pending");
    };
    assert!(
        reverted
            .reason
            .to_string()
            .contains("pending until the end of the rollup block"),
        "{}",
        reverted.reason
    );
}

#[test]
fn slashing_drops_pending_rotation_away_from_slashed_address() {
    let (_, user, mut runner) = setup(0);
    let da = MockAddress::new([11; 32]);
    let new_da = MockAddress::new([12; 32]);
    register(&mut runner, &user, da);
    let owner = user.address();
    runner.__apply_to_state(move |state| {
        let mut reg = SequencerRegistry::<S>::default();
        reg.pending_da_address_update
            .set(
                &PendingDaAddressUpdate {
                    sequencer: owner,
                    old_da_address: da,
                    new_da_address: new_da,
                },
                state,
            )
            .unwrap();
        reg.slash_sequencer(&da, state);
        assert!(reg.pending_da_address_update.get(state).unwrap().is_none());
    });
}

#[test]
fn restarting_runtime_across_activation_preserves_execution() {
    let (preferred, user, mut runner) = setup(2);
    let time = sov_modules_api::da::Time::from_secs(1_000);
    runner.config.freeze_time = Some(time.clone());
    let old_da = MockAddress::new([11; 32]);
    let new_da = MockAddress::new([12; 32]);
    register(&mut runner, &user, old_da);
    let mut nonces = runner.nonces().clone();
    for message in [
        Message::UpdateDaAddress {
            old_da_address: old_da,
            new_da_address: new_da,
        },
        Message::Deposit {
            da_address: new_da,
            amount: Amount::new(123),
        },
    ] {
        let (blobs, _) = TestRunner::<RT, S>::batches_to_blobs(
            vec![(
                BatchType(vec![
                    user.create_plain_message::<RT, RuntimeRegistry<S>>(message)
                ]),
                preferred.da_address,
            )],
            &mut nonces,
        );
        let storage = runner.storage_manager().create_prover_storage();
        let header = MockBlockHeader::new(runner.true_slot_number().get() + 1, time.clone());
        let restarted = TestStfBlueprint::<RT, S>::new();
        let mut replay_blobs = blobs.clone();
        let replay = restarted.apply_slot(
            runner.state_root(),
            storage,
            Default::default(),
            &header,
            replay_blobs.as_iters(),
            ExecutionContext::Node,
        );
        let (receipt, _) = runner.execute(blobs);
        assert_eq!(replay.state_root, *runner.state_root());
        assert_eq!(
            serde_json::to_value(&replay.batch_receipts).unwrap(),
            serde_json::to_value(&receipt.batch_receipts).unwrap()
        );
        assert!(receipt.last_tx_receipt().receipt.is_successful());
    }
}
