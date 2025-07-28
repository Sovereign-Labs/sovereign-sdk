use std::convert::Infallible;

use serde::Serialize;
use sov_bank::{config_gas_token_id, Bank};
use sov_chain_state::ChainState;
use sov_mock_zkvm::MockCodeCommitment;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{
    AggregatedProofPublicData, Amount, ApiStateAccessor, CodeCommitment, SerializedAggregatedProof,
    Spec, Storage,
};
use sov_modules_rollup_blueprint::proof_sender::serialize_proof_blob_with_metadata;
use sov_prover_incentives::ProverIncentives;
use sov_rollup_interface::common::SlotNumber;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_zk_runtime, AsUser, TestProver, TestSpec, TestUser, TransactionType,
};
use sov_value_setter::ValueSetterConfig;

pub(crate) type S = sov_test_utils::TestSpec;
pub(crate) type TestProverIncentives = ProverIncentives<S>;
pub(crate) type RT = TestRuntime<S>;
pub(crate) const MOCK_CODE_COMMITMENT: MockCodeCommitment = MockCodeCommitment([0u8; 8]);

generate_zk_runtime!(TestRuntime <= value_setter: sov_value_setter::ValueSetter<S>);

/// Returns the minimal bond required to register a prover at the current slot.
pub fn minimal_bond(runner: &TestRunner<TestRuntime<S>, S>) -> u128 {
    runner
        .query_visible_state(|state| {
            TestProverIncentives::default()
                .get_minimum_bond(state)
                .unwrap_infallible()
                .unwrap()
        })
        .0
}

pub(crate) fn setup_with_custom_runtime(
    runtime: TestRuntime<S>,
) -> (TestRunner<RT, S>, TestProver<TestSpec>, TestUser<S>) {
    let minimal_genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(1);
    let unbonded_user = minimal_genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let prover = minimal_genesis_config.initial_prover.clone();
    let genesis_config = GenesisConfig::from_minimal_config(
        minimal_genesis_config.into(),
        ValueSetterConfig {
            admin: prover.user_info.address(),
        },
    );
    let runner = TestRunner::new_with_genesis(genesis_config.into_genesis_params(), runtime);

    (runner, prover, unbonded_user)
}

/// Same as [`setup_with_custom_runtime`] but uses the default runtime.
pub(crate) fn setup() -> (TestRunner<RT, S>, TestProver<TestSpec>, TestUser<S>) {
    setup_with_custom_runtime(TestRuntime::default())
}

#[allow(clippy::type_complexity)]
pub(crate) fn build_proof(
    state: &mut ApiStateAccessor<S>,
    initial_slot: SlotNumber,
    end_slot: SlotNumber,
    prover_address: <S as Spec>::Address,
) -> Result<
    AggregatedProofPublicData<
        <S as Spec>::Address,
        <S as Spec>::Da,
        <<S as Spec>::Storage as Storage>::Root,
    >,
    Infallible,
> {
    let chain_state = ChainState::<S>::default();
    let genesis_hash = chain_state
        .get_genesis_hash(state)
        .unwrap()
        .expect("Genesis hash must be set");
    let initial_transition = chain_state
        .slot_at_height(initial_slot, state)
        .unwrap()
        .unwrap();
    let end_transition = chain_state
        .get_historical_transition_dangerous(end_slot, state)
        .unwrap()
        .unwrap();

    Ok(AggregatedProofPublicData {
        initial_slot_number: initial_slot,
        final_slot_number: end_slot,
        initial_state_root: genesis_hash,
        genesis_state_root: genesis_hash,
        final_state_root: *end_transition.post_state_root(),
        initial_slot_hash: *initial_transition.slot_hash(),
        final_slot_hash: *end_transition.slot().slot_hash(),
        code_commitment: CodeCommitment(MOCK_CODE_COMMITMENT.0.to_vec()),
        rewarded_addresses: vec![prover_address],
    })
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

pub(crate) fn serialize_proof<T: Serialize>(agg_proof: T) -> Vec<u8> {
    let proof = sov_mock_zkvm::MockZkvmHost::create_serialized_proof(true, agg_proof);
    let serialized_proof = SerializedAggregatedProof {
        raw_aggregated_proof: proof,
    };

    borsh::to_vec(&serialize_proof_blob_with_metadata::<S>(serialized_proof).unwrap()).unwrap()
}
