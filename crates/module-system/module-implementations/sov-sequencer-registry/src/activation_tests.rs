use sov_mock_da::{MockAddress, MockBlob, MockBlockHeader};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::prelude::serde_json;
use sov_modules_api::{
    Amount, BlockHooks, ExecutionContext, Spec, StateCheckpoint, StateTransitionFunction, TxEffect,
    VersionReader,
};
use sov_state::Storage;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{SequencerRegistry as RuntimeRegistry, TestRunner};
use sov_test_utils::storage::ForklessStorageManager;
use sov_test_utils::{
    generate_optimistic_runtime, AsUser, BatchType, TestSequencer, TestSpec, TestStfBlueprint,
    TestUser, TransactionTestCase, ZkTestSpec,
};

use crate::{BalanceState, SequencerRegistry};

type S = TestSpec;
generate_optimistic_runtime!(ActivationRuntime <=);
type RT = ActivationRuntime<S>;
type Message = sov_sequencer_registry::CallMessage<S>;

const BOND: Amount = Amount::new(100_000_000_000);
const HEIGHT_OVERRIDE: &str = "SOV_TEST_CONST_OVERRIDE_SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT";
const LIST_OVERRIDE: &str = "SOV_TEST_CONST_OVERRIDE_SEQUENCER_DA_ROTATION_UPGRADE_ADDRESSES";

fn setup(height: u64) -> (TestSequencer<S>, TestUser<S>, TestRunner<RT, S>) {
    std::env::set_var(HEIGHT_OVERRIDE, height.to_string());
    std::env::set_var(LIST_OVERRIDE, "");
    let config = HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let preferred = config.initial_sequencer.clone();
    let user = config.additional_accounts()[0].clone();
    let genesis = GenesisConfig::from_minimal_config(config.into());
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    (preferred, user, runner)
}

fn set_list(addresses: &[MockAddress]) {
    std::env::set_var(
        LIST_OVERRIDE,
        addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
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
fn activation_preserves_pending_withdrawal_and_tracks_each_owner() {
    let (preferred, user, mut runner) = setup(3);
    let da = MockAddress::new([11; 32]);
    let rotated = MockAddress::new([12; 32]);
    let reg = SequencerRegistry::<S>::default();
    runner.query_visible_state(|state| {
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), None);
    });
    register(&mut runner, &user, da);
    assert!(execute(
        &mut runner,
        &user,
        Message::InitiateWithdrawal { da_address: da }
    )
    .is_successful());
    let before = runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 2);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), None);
        reg.known_sequencers.get(&da, state).unwrap().unwrap()
    });
    assert!(matches!(
        before.balance_state,
        BalanceState::PendingWithdrawal { .. }
    ));

    set_list(&[da, preferred.da_address]);
    runner.advance_slots(1);
    let registration_id = runner.query_visible_state(|state| {
        assert_eq!(state.rollup_height_to_access().get(), 3);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(2));
        assert_eq!(
            reg.known_sequencers.get(&da, state).unwrap(),
            Some(before.clone())
        );
        assert_eq!(
            reg.get_preferred_sequencer(state).unwrap(),
            Some((preferred.da_address, preferred.user_info.address()))
        );
        let current = reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .unwrap();
        let expected_id = if da.to_string() < preferred.da_address.to_string() {
            1
        } else {
            2
        };
        assert_eq!(current.registration_id, expected_id);
        assert_eq!(current.current_da_address, da);
        current.registration_id
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
        assert_eq!(state.rollup_height_to_access().get(), 4);
        assert_eq!(
            reg.known_sequencers.get(&rotated, state).unwrap(),
            Some(before)
        );
        let current = reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .unwrap();
        assert_eq!(current.registration_id, registration_id);
        assert_eq!(current.current_da_address, rotated);
    });
    assert!(matches!(
        execute(
            &mut runner,
            &user,
            Message::Register {
                da_address: da,
                amount: BOND
            }
        ),
        TxEffect::Reverted(_)
    ));
}

#[test]
fn preferred_rotation_is_rejected_before_activation_and_applied_at_activation() {
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
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), None);
        assert!(reg.pending_da_address_update.get(state).unwrap().is_none());
        assert!(reg
            .known_sequencers
            .get(&preferred.da_address, state)
            .unwrap()
            .is_some());
    });
    set_list(&[preferred.da_address]);
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
    runner.config.sequencer_da_address = new_da;
    runner.advance_slots(1);
    runner.query_visible_state(|state| {
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(1));
    });
}

#[test]
fn activation_does_not_recreate_registration_slashed_during_blob_selection() {
    let (preferred, user, mut runner) = setup(2);
    let da = MockAddress::new([11; 32]);
    register(&mut runner, &user, da);
    set_list(&[da, preferred.da_address]);
    let mut nonces = runner.nonces().clone();
    let (mut blobs, _) = TestRunner::<RT, S>::batches_to_blobs(
        vec![(BatchType(Vec::new()), preferred.da_address)],
        &mut nonces,
    );
    blobs
        .batch_blobs
        .push(MockBlob::new_with_hash(vec![0xff; 100], da));
    runner.execute(blobs);
    runner.query_visible_state(|state| {
        let reg = SequencerRegistry::<S>::default();
        assert_eq!(state.rollup_height_to_access().get(), 2);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(1));
        assert!(reg.known_sequencers.get(&da, state).unwrap().is_none());
        assert!(reg
            .current_da_address_by_sequencer
            .get(&user.address(), state)
            .unwrap()
            .is_none());
    });
}

#[test]
#[should_panic(expected = "Sequencer owners must consolidate")]
fn activation_rejects_unconsolidated_owners() {
    let (preferred, user, mut runner) = setup(3);
    let da_a = MockAddress::new([11; 32]);
    let da_b = MockAddress::new([12; 32]);
    register(&mut runner, &user, da_a);
    register(&mut runner, &user, da_b);
    set_list(&[preferred.da_address, da_a, da_b]);
    runner.advance_slots(1);
}

#[test]
#[should_panic(expected = "Duplicate sequencer DA upgrade address")]
fn activation_rejects_duplicate_da_addresses() {
    let (preferred, _, mut runner) = setup(1);
    set_list(&[preferred.da_address, preferred.da_address]);
    runner.advance_slots(1);
}

#[test]
#[should_panic(expected = "Invalid sequencer DA upgrade address")]
fn activation_rejects_malformed_da_addresses() {
    let (_, _, mut runner) = setup(1);
    std::env::set_var(LIST_OVERRIDE, "invalid-da-address");
    runner.advance_slots(1);
}

#[test]
fn genesis_activation_initializes_tracking_and_repeated_hook_preserves_ids() {
    let (preferred, _, mut runner) = setup(0);
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
    let root = *runner.state_root();
    runner.__apply_to_state(|state| {
        let mut reg = SequencerRegistry::<S>::default();
        reg.begin_rollup_block_hook(&root, state);
        reg.begin_rollup_block_hook(&root, state);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(1));
    });
}

#[test]
#[should_panic(expected = "tracking was not initialized at activation")]
fn block_after_activation_rejects_missing_tracking() {
    let (_, _, mut runner) = setup(0);
    runner.__apply_to_state(|state| {
        SequencerRegistry::<S>::default()
            .registration_id_counter
            .delete(state)
            .unwrap();
    });
    runner.advance_slots(1);
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
fn activation_bootstrap_executes_against_verifier_storage() {
    let (preferred, _, runner) = setup(1);
    set_list(&[preferred.da_address]);
    let pre_root = *runner.state_root();
    let storage = runner.storage_manager().create_prover_storage();
    let mut runtime = RT::default();
    let mut state = StateCheckpoint::<S>::new(storage.clone(), &runtime.kernel());
    state.advance_visible_slot_number(std::num::NonZeroU8::new(1).unwrap());
    SequencerRegistry::<S>::default().begin_rollup_block_hook(&pre_root, &mut state);
    let (accesses, _, witness) = state.freeze();
    let (native_root, _) = storage
        .compute_state_update(accesses, &witness, pre_root)
        .unwrap();

    let verifier = <ZkTestSpec as Spec>::Storage::new();
    let mut zk_runtime = ActivationRuntime::<ZkTestSpec>::default();
    let mut zk_state = StateCheckpoint::<ZkTestSpec>::with_witness(
        verifier.clone(),
        witness,
        &zk_runtime.kernel(),
    );
    zk_state.advance_visible_slot_number(std::num::NonZeroU8::new(1).unwrap());
    SequencerRegistry::<ZkTestSpec>::default().begin_rollup_block_hook(&pre_root, &mut zk_state);
    let (accesses, _, witness) = zk_state.freeze();
    let (verifier_root, _) = verifier
        .compute_state_update(accesses, &witness, pre_root)
        .unwrap();
    assert_eq!(native_root, verifier_root);
}

#[test]
fn restarting_runtime_across_activation_preserves_execution() {
    let (preferred, user, mut runner) = setup(2);
    let time = sov_modules_api::da::Time::from_secs(1_000);
    runner.config.freeze_time = Some(time.clone());
    let old_da = MockAddress::new([11; 32]);
    let new_da = MockAddress::new([12; 32]);
    register(&mut runner, &user, old_da);
    set_list(&[preferred.da_address, old_da]);
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

#[test]
#[should_panic(expected = "Genesis activation requires an empty sequencer DA upgrade list")]
fn genesis_activation_rejects_upgrade_list() {
    std::env::set_var(HEIGHT_OVERRIDE, "0");
    set_list(&[MockAddress::new([11; 32])]);
    let config = HighLevelOptimisticGenesisConfig::<S>::generate();
    let genesis = GenesisConfig::from_minimal_config(config.into());
    let _ = TestRunner::<RT, S>::new_with_genesis(genesis.into_genesis_params(), RT::default());
}

#[test]
fn activation_with_no_surviving_registrations_marks_initialization_complete() {
    let (preferred, _, mut runner) = setup(1);
    set_list(&[preferred.da_address]);
    let mut nonces = runner.nonces().clone();
    let (mut blobs, _) = TestRunner::<RT, S>::batches_to_blobs(Vec::new(), &mut nonces);
    blobs.batch_blobs.push(MockBlob::new_with_hash(
        vec![0xff; 100],
        preferred.da_address,
    ));
    runner.execute(blobs);
    runner.advance_slots(1);
    runner.query_visible_state(|state| {
        let reg = SequencerRegistry::<S>::default();
        assert_eq!(state.rollup_height_to_access().get(), 2);
        assert_eq!(reg.registration_id_counter.get(state).unwrap(), Some(0));
        assert!(reg
            .current_da_address_by_sequencer
            .get(&preferred.user_info.address(), state)
            .unwrap()
            .is_none());
    });
}
