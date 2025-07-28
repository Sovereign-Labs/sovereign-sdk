use sov_state::{ProvableNamespace, StateRoot};
use sov_test_utils::generate_runtime;

use crate::kernel_interactions::{
    last_state_root_closure, HighLevelOptimisticGenesisConfig, TestClosureArgs, TestUser,
    TestVisibleHashModule, S,
};

generate_runtime! {
    name: TestVisibleHashRuntime,
    modules: [visible_hash_module: TestVisibleHashModule<S>],
    operating_mode: sov_modules_api::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestVisibleHashRuntime<S>>,
    auth_call_wrapper: |call| call,
}

type TestRunner<RT> = sov_test_utils::runtime::TestRunner<RT, S>;

type RT = TestVisibleHashRuntime<S>;

fn setup() -> (TestUser<S>, TestRunner<RT>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (user, runner)
}

/// Tests that the visible kernel hash updates for each slot for the basic Kernel
#[test]
fn visible_hash_basic_kernel() {
    let (_, mut runner) = setup();
    let genesis_hash = *runner.state_root();

    const NUM_SLOTS: u64 = 10;
    let state_root_delay_blocks: u64 =
        sov_modules_api::macros::config_value!("STATE_ROOT_DELAY_BLOCKS");
    let mut roots = Vec::new();
    roots.push(genesis_hash);
    for prev_block_number in 0..=(NUM_SLOTS + state_root_delay_blocks) {
        last_state_root_closure(
            &mut |TestClosureArgs {
                      prev_finalize_hook_hash,
                      prev_slot_hash,
                      current_slot_hash,
                      begin_slot_hash,
                      ..
                  }| {
                assert_eq!(
                    prev_finalize_hook_hash,
                    begin_slot_hash.unwrap(),
                    "The previous finalize hash should always match the begin slot hash"
                );

                roots.push(current_slot_hash);

                assert_eq!(
                    begin_slot_hash.unwrap(),
                    roots[prev_block_number.saturating_sub(state_root_delay_blocks) as usize],
                    "The current slot hash should match the expected hash"
                );

                let current_block_number = prev_block_number + 1;
                if current_block_number > state_root_delay_blocks {
                    assert_ne!(
                        current_slot_hash, prev_slot_hash,
                        "The slot hash should always update"
                    );
                    assert_ne!(
                        current_slot_hash.namespace_root(ProvableNamespace::Kernel),
                        prev_slot_hash.namespace_root(ProvableNamespace::Kernel),
                        "The kernel hash should always update in the basic kernel"
                    );

                    assert_ne!(
                        current_slot_hash.namespace_root(ProvableNamespace::User),
                        prev_slot_hash.namespace_root(ProvableNamespace::User),
                        "The user hash should always update in the basic kernel"
                    );
                }
            },
            &mut runner,
            1,
        );
    }
}
