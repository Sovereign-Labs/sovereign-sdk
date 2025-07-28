use std::convert::Infallible;

use sov_attester_incentives::Attestation;
use sov_bank::{config_gas_token_id, Bank};
use sov_chain_state::ChainState;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvmHost;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::{
    Amount, ApiStateAccessor, DaSpec, ProofOutcome, SerializedAttestation, SerializedChallenge,
    Spec, StateTransitionPublicData,
};
use sov_modules_rollup_blueprint::proof_sender::{
    serialize_attestation_blob_with_metadata, serialize_challenge_blob_with_metadata,
};
use sov_rollup_interface::common::{IntoSlotNumber, SlotNumber};
use sov_state::{Storage, StorageProof};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{AttesterIncentives, TestRunner};
use sov_test_utils::{
    assert_matches, generate_optimistic_runtime, AsUser, AtomicAmount, ProofInput, ProofTestCase,
    TestAttester, TestChallenger, TestUser, TransactionType,
};
use sov_value_setter::ValueSetterConfig;

pub(crate) type S = sov_test_utils::TestSpec;

pub(crate) type TestAttesterIncentives = AttesterIncentives<S>;

pub(crate) type RT = TestRuntime<S>;

generate_optimistic_runtime!(TestRuntime <= value_setter: sov_value_setter::ValueSetter<S>);

pub type SetupParams = (
    TestRunner<RT, S>,
    TestAttester<S>,
    TestChallenger<S>,
    TestUser<S>,
);

/// Returns the minimal bond required to register an attester at the current slot.
pub fn minimal_attester_bond(runner: &TestRunner<TestRuntime<S>, S>) -> u128 {
    runner.query_visible_state(|state| {
        TestAttesterIncentives::default()
            .get_minimal_attester_bond_value(state)
            .0
    })
}

/// Returns the minimal bond required to register a challenger at the current slot.
pub fn minimal_challenger_bond(runner: &TestRunner<TestRuntime<S>, S>) -> u128 {
    runner.query_visible_state(|state| {
        TestAttesterIncentives::default()
            .get_minimal_challenger_bond_value(state)
            .0
    })
}

pub(crate) fn setup_with_custom_runtime(runtime: TestRuntime<S>) -> SetupParams {
    // Generate a genesis config, then overwrite the attester key/address with ones that
    // we know. We leave the other values untouched.
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let genesis_attester = genesis_config.initial_attester.clone();

    let attester_address = genesis_attester.user_info.address();
    let attester_bond = genesis_attester.bond;
    let attester_balance = genesis_attester.user_info.available_gas_balance;

    let genesis_challenger = genesis_config.initial_challenger.clone();

    let additional_account = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: additional_account.address(),
        },
    );

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), runtime);

    runner.query_visible_state(|state| {
        // Check that the attester account is bonded
        assert_eq!(
            TestAttesterIncentives::default()
                .bonded_attesters
                .get(&attester_address, state)
                .unwrap(),
            Some(attester_bond),
            "The genesis attester should be bonded"
        );

        // Check the balance of the attester is equal to the free balance
        assert_eq!(
            TestRunner::<RT, S>::bank_gas_balance(&attester_address, state),
            Some(attester_balance),
            "The balance of the attester should be equal to the free balance"
        );
    });

    (
        runner,
        genesis_attester,
        genesis_challenger,
        additional_account,
    )
}

/// Helper that sets up the tests and checks that the genesis state is valid.
pub(crate) fn setup() -> SetupParams {
    setup_with_custom_runtime(RT::default())
}

pub(crate) fn consume_gas_tx_for_signer(signer: &TestUser<S>) -> TransactionType<RT, S> {
    let recipient = TestUser::<S>::generate(Amount::ZERO);
    signer.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
        to: recipient.address(),
        coins: sov_bank::Coins {
            amount: Amount::new(1000),
            token_id: config_gas_token_id(),
        },
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn build_proof(
    state: &mut ApiStateAccessor<S>,
    rollup_height_to_attest: u64,
    user_address: &<S as Spec>::Address,
) -> Result<
    Attestation<
        <MockDaSpec as DaSpec>::SlotHash,
        <<S as Spec>::Storage as Storage>::Root,
        StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
    >,
    Infallible,
> {
    let height_to_attest = rollup_height_to_attest.to_slot_number();
    let chain_state = ChainState::<S>::default();

    // Get the values for the transition being attested
    let current_transition = chain_state
        .get_historical_transition_dangerous(height_to_attest, state)?
        .unwrap();

    let prev_root = if rollup_height_to_attest == 1 {
        chain_state.get_genesis_hash(state)?
    } else {
        chain_state
            .slot_at_height(height_to_attest, state)?
            .map(|slot| *slot.prev_state_root())
    }
    .unwrap();

    let mut archival_state = state
        .get_archival_state(RollupHeight::new(rollup_height_to_attest))
        .unwrap();

    let proof_of_bond = TestAttesterIncentives::default()
        .bonded_attesters
        .get_with_proof(user_address, &mut archival_state)
        .unwrap();

    Ok(Attestation {
        initial_state_root: prev_root,
        slot_hash: *current_transition.slot().slot_hash(),
        post_state_root: *current_transition.post_state_root(),
        proof_of_bond: sov_modules_api::optimistic::ProofOfBond {
            claimed_slot_number: height_to_attest,
            proof: proof_of_bond,
        },
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn make_attestation_blob(
    attestation: Attestation<
        <MockDaSpec as DaSpec>::SlotHash,
        <<S as Spec>::Storage as Storage>::Root,
        StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
    >,
) -> Vec<u8> {
    let serialized_attestation = SerializedAttestation::from_attestation(&attestation).unwrap();

    borsh::to_vec(&serialize_attestation_blob_with_metadata::<S>(serialized_attestation).unwrap())
        .unwrap()
}

pub(crate) fn create_test_case(
    genesis_attester: TestAttester<S>,
    serialized_attestation: Vec<u8>,
    initial_balance: Amount,
    reward: AtomicAmount,
) -> ProofTestCase<S> {
    let attester_address = genesis_attester.user_info.address();

    ProofTestCase {
        input: ProofInput(serialized_attestation),
        assert: Box::new(move |result, state| {
            assert_matches!(
                result.proof_receipt.unwrap().outcome,
                ProofOutcome::Valid { .. }
            );

            assert_eq!(
                TestAttesterIncentives::default()
                    .bonded_attesters
                    .get(&attester_address, state)
                    .unwrap(),
                Some(genesis_attester.bond),
                "Bonded amount should not have changed"
            );

            assert_eq!(
                TestRunner::<RT, S>::bank_gas_balance(&attester_address, state).unwrap(),
                initial_balance
                    .checked_sub(result.gas_value_used)
                    .unwrap()
                    .checked_add(
                        TestAttesterIncentives::default()
                            .burn_rate()
                            .apply(reward.get())
                    )
                    .unwrap()
            );
        }),
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn build_challenge(
    state: &mut ApiStateAccessor<S>,
    challenge_slot: SlotNumber,
    prover_address: <S as Spec>::Address,
) -> Result<
    StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    >,
    Infallible,
> {
    let chain_state = ChainState::<S>::default();
    // Get the values for the transition being attested
    let current_transition = chain_state
        .get_historical_transition_dangerous(challenge_slot, state)?
        .unwrap();

    let challenge: StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    > = StateTransitionPublicData {
        initial_state_root: *current_transition.slot().prev_state_root(),
        final_state_root: *current_transition.post_state_root(),
        slot_hash: *current_transition.slot().slot_hash(),
        prover_address,
    };

    Ok(challenge)
}

#[allow(clippy::type_complexity)]
pub(crate) fn make_challenge_blob(
    challenge: StateTransitionPublicData<
        <S as Spec>::Address,
        MockDaSpec,
        <<S as Spec>::Storage as Storage>::Root,
    >,
    is_valid: bool,
    challenge_slot: SlotNumber,
) -> Vec<u8> {
    let serialized_challenge = MockZkvmHost::create_serialized_proof(is_valid, challenge);
    let serialized_challenge = SerializedChallenge {
        raw_challenge: serialized_challenge,
    };

    borsh::to_vec(
        &serialize_challenge_blob_with_metadata::<S>(serialized_challenge, challenge_slot).unwrap(),
    )
    .unwrap()
}
