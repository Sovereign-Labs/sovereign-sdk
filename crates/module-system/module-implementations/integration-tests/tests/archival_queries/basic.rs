use jsonrpsee::tokio;
use sov_chain_state::ChainState;
use sov_modules_api::da::Time;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_test_utils::generate_optimistic_runtime_with_kernel;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{ApiGetStateData, ApiPath};

use crate::archival_queries::{TestRunner, S};

generate_optimistic_runtime_with_kernel!(TestArchivalBasicRuntime <= kernel_type: sov_kernels::basic::BasicKernel<'a, S>,);

type RT = TestArchivalBasicRuntime<S>;

fn setup() -> TestRunner<RT> {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
}

/// Check that the state is automatically updated at every height
#[tokio::test(flavor = "multi_thread")]
async fn query_time_basic_kernel() {
    let mut runner = setup();

    let client = runner.setup_rest_api_server().await;

    const SLOTS_TO_ADVANCE: u64 = 10;

    let mut prev_time = runner.query_state(|state| {
        ChainState::<S>::default()
            .get_time(state)
            .unwrap_infallible()
    });

    for i in 1..SLOTS_TO_ADVANCE {
        runner.advance_slots(1);

        let current_time = runner.query_state(|state| {
            ChainState::<S>::default()
                .get_time(state)
                .unwrap_infallible()
        });

        let api_time = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state").with_default_state_path("time"),
                &client,
            )
            .await
            .value
            .unwrap();

        let api_time_at_height = runner
            .query_api_response::<ApiGetStateData<Time>>(
                &ApiPath::query_module("chain-state")
                    .with_default_state_path("time")
                    .with_rollup_height(i),
                &client,
            )
            .await
            .value
            .unwrap();

        assert_eq!(current_time, api_time);
        assert_eq!(current_time, api_time_at_height);

        assert!(current_time > prev_time);

        prev_time = current_time;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn query_invalid_rollup_height_returns_error() {
    let mut runner = setup();

    const SLOTS_TO_ADVANCE: u64 = 10;

    let client = runner.setup_rest_api_server().await;

    runner.advance_slots(SLOTS_TO_ADVANCE as usize);

    // Using the API without the rollup height parameter should return the value at the current visible height
    // It should be possible to retrieve the time at the height 10.
    runner
        .query_api_response::<ApiGetStateData<Time>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("time")
                .with_rollup_height(SLOTS_TO_ADVANCE),
            &client,
        )
        .await
        .value
        .unwrap();

    let api_response = runner
        .query_api_response::<ApiGetStateData<Time>>(
            &ApiPath::query_module("chain-state")
                .with_default_state_path("time")
                .with_rollup_height(SLOTS_TO_ADVANCE + 1),
            &client,
        )
        .await;

    assert_eq!(api_response.errors.len(), 1);
    let error = api_response.errors.first().unwrap();
    assert_eq!(error.status, 404);
    assert_eq!(error.title, "invalid rollup height");
    assert_eq!(
        error.details.get("message").unwrap(),
        "Impossible to get the rollup state at the specified height. Please ensure you have queried the correct height."
    );
}
