use std::collections::HashMap;

use jsonrpsee::tokio;
use sov_chain_state::{ChainState, SlotInformation};
use sov_mock_da::MockBlob;
use sov_modules_api::da::Time;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{CryptoSpec, Spec};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{ApiGetStateData, ApiPath};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, BatchType, SequencerInfo, SoftConfirmationBlobInfo,
    TestSequencer,
};

use super::S;
use crate::archival_queries::TestRunner;

generate_optimistic_runtime_with_kernel!(TestArchivalSoftConfirmationsRuntime <= kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,);

type RT = TestArchivalSoftConfirmationsRuntime<S>;
type SlotConfigInfo<SequencerInfo> = Vec<SequencerInfo>;

fn setup() -> (TestSequencer<S>, TestRunner<RT>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let sequencer = genesis_config.initial_sequencer.clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (sequencer, runner)
}

/// Builds a [`RelevantBlobs`] struct from a list of [`BlobConfigInfo`]s.
/// This struct populates the batches with simple [`ValueSetter`] messages. One
/// can specify special sequencer addresses for each batch.
pub fn build_soft_confirmation_blobs(
    slot_info: &SlotConfigInfo<(TestSequencer<S>, SequencerInfo)>,
    nonces: &mut HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    runner: &mut TestRunner<RT>,
) -> RelevantBlobs<MockBlob> {
    let mut batches = Vec::new();

    for (sequencer, additional_info) in slot_info {
        batches.push(SoftConfirmationBlobInfo {
            batch_type: BatchType(vec![]),
            sequencer_address: sequencer.da_address,
            sequencer_info: additional_info.clone(),
        });
    }

    TestRunner::<RT>::soft_confirmation_batches_to_blobs(batches, nonces)
}

/// Check that we can query the kernel state of versioned accessors at any height.
#[tokio::test(flavor = "multi_thread")]
async fn archival_queries_soft_confirmations() {
    let (_, mut runner) = setup();

    let client = runner.setup_rest_api_server().await;

    let deferred_slots_count: u64 = config_value!("DEFERRED_SLOTS_COUNT");

    let init_time = runner
        .query_api_response::<ApiGetStateData<Time>>(
            &ApiPath::query_module("chain-state").with_default_state_path("time"),
            &client,
        )
        .await
        .value
        .unwrap();

    let chain_state_time = runner.query_state(|state| {
        ChainState::<S>::default()
            .get_time(state)
            .unwrap_infallible()
    });

    assert_eq!(init_time, chain_state_time);

    // Up until `DEFERRED_SLOTS_COUNT` the time should be the same as the genesis time because
    // the visible height does not change.
    for i in 1..deferred_slots_count {
        runner.advance_slots(1);

        assert_eq!(runner.visible_slot_number(), 0);
        assert_eq!(runner.true_slot_number(), i);

        let chain_state_time = runner.query_state(|state| {
            ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible()
        });

        assert!(chain_state_time > init_time);

        // Using the API without the rollup height parameter should return the value at the current visible height
        let time = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state").with_default_state_path("time"),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(time, init_time);

        // We query the visible height from the rollup state and we try to query the time at the same height and
        // see if we get the same time as without the visible height parameter
        let visible_slot_number = runner
            .query_api_response::<ApiGetStateData<u64>>(
                &ApiPath::query_module("chain-state")
                    .with_default_state_path("next-visible-rollup-height"),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(visible_slot_number, 0);

        let time_visible_slot_number = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state")
                    .with_default_state_path("time")
                    .with_rollup_height(visible_slot_number),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(time_visible_slot_number, time);

        // We can query any height using the API with the rollup height parameter
        let current_time = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state")
                    .with_default_state_path("time")
                    .with_rollup_height(i),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(current_time, chain_state_time);
    }

    // Advance the visible height to the next slot. The visible height should update to 1. The time should be updated.
    runner.advance_slots(1);

    assert_eq!(runner.visible_slot_number(), 1);

    let chain_state_time = runner
        .query_state_at_height(1, |state| {
            ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible()
        })
        .unwrap();

    let current_time_api = runner
        .query_api_response::<ApiGetStateData<Time>>(
            &ApiPath::query_module("chain-state").with_default_state_path("time"),
            &client,
        )
        .await
        .value
        .unwrap();

    let time_height_0 = runner
        .query_api_response::<ApiGetStateData<Time>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("time")
                .with_rollup_height(0),
            &client,
        )
        .await
        .value
        .unwrap();

    assert_eq!(chain_state_time, current_time_api);
    assert_eq!(time_height_0, init_time);

    assert!(current_time_api > init_time);
}

/// Check that we can query all the intermediary states of the chain even when the visible height skips some slots.
#[tokio::test(flavor = "multi_thread")]
async fn intermediary_state_queries_soft_confirmations() {
    let (sequencer, mut runner) = setup();

    let deferred_slots_count: u64 = config_value!("DEFERRED_SLOTS_COUNT");

    let client = runner.setup_rest_api_server().await;

    runner.advance_slots((deferred_slots_count - 1) as usize);

    assert_eq!(runner.true_slot_number(), deferred_slots_count - 1);
    assert_eq!(runner.visible_slot_number(), 0);

    let blobs = build_soft_confirmation_blobs(
        &vec![(
            sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: deferred_slots_count - 1,
                sequence_number: 0,
            },
        )],
        &mut HashMap::new(),
        &mut runner,
    );

    runner.execute::<_>(blobs);

    assert_eq!(runner.visible_slot_number(), deferred_slots_count - 1);

    let mut prev_time = runner
        .query_state_at_height(0, |state| {
            ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible()
        })
        .unwrap();

    for i in 1..deferred_slots_count {
        let chain_state_time = runner
            .query_state_at_height(i, |state| {
                ChainState::<S>::default()
                    .get_time(state)
                    .unwrap_infallible()
            })
            .unwrap();

        // Using the API without the rollup height parameter should return the value at the current visible height
        let time = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state")
                    .with_default_state_path("time")
                    .with_rollup_height(i),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(time, chain_state_time);
        assert!(time > prev_time);

        prev_time = time;
    }
}

/// Ensure that querying versioned state vectors works as expected.
/// Ie, that we can query the current value, the previous value, and any value below the specific height visible by the accessor.
#[tokio::test(flavor = "multi_thread")]
async fn query_versioned_vector() {
    let (_, mut runner) = setup();

    let client = runner.setup_rest_api_server().await;

    let deferred_slots_count: u64 = config_value!("DEFERRED_SLOTS_COUNT");

    runner.advance_slots((deferred_slots_count) as usize);

    assert_eq!(runner.true_slot_number(), deferred_slots_count);
    assert_eq!(runner.visible_slot_number(), 1);

    // We can query the current value of the versioned state vector
    let slot_height_1 = runner.query_state(|state| {
        ChainState::<S>::default()
            .slot_at_height(1, state)
            .unwrap_infallible()
            .unwrap()
    });

    // We can query the slot number 1 because the visible height is 1.
    let api_slot_height_1 = runner
        .query_api_response::<ApiGetStateData<SlotInformation<S>>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("slots")
                .get_item_number(1),
            &client,
        )
        .await
        .value
        .unwrap();

    assert_eq!(slot_height_1, api_slot_height_1);

    // We cannot query the slot number 2 because the visible height is 1.
    let api_slot_height_2_err = runner
        .query_api_response::<ApiGetStateData<SlotInformation<S>>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("slots")
                .get_item_number(2),
            &client,
        )
        .await;

    assert_eq!(api_slot_height_2_err.errors.len(), 1);
    let error = api_slot_height_2_err.errors.first().unwrap();
    assert_eq!(error.status, 404);
    assert_eq!(error.title, "slots '2' not found");
    assert_eq!(error.details.get("id").unwrap(), "2");

    let slot_height_2 = runner.query_state(|state| {
        ChainState::<S>::default()
            .slot_at_height(2, state)
            .unwrap_infallible()
            .unwrap()
    });

    // But we can query the slot number 2 at height `deferred_slots_count`.
    let api_slot_height_2 = runner
        .query_api_response::<ApiGetStateData<SlotInformation<S>>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("slots")
                .get_item_number(2)
                .with_rollup_height(deferred_slots_count),
            &client,
        )
        .await
        .value
        .unwrap();

    assert_eq!(slot_height_2, api_slot_height_2);
}
