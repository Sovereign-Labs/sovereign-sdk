use sov_modules_api::macros::config_value;
use sov_state::{ProvableNamespace, StateRoot};
use sov_test_utils::{generate_runtime, TestSequencer};

use crate::kernel_interactions::{
    last_state_root_closure, HighLevelOptimisticGenesisConfig, TestClosureArgs, TestRunner,
    TestUser, TestVisibleHashModule, S,
};

generate_runtime! {
    name: TestVisibleHashRuntime,
    modules: [visible_hash_module: TestVisibleHashModule<S>],
    operating_mode: sov_modules_api::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestVisibleHashRuntime<S>>,
    auth_call_wrapper: |call| call,
}

type RT = TestVisibleHashRuntime<S>;

fn setup() -> (TestUser<S>, TestSequencer<S>, TestRunner<RT>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let sequencer = genesis_config.initial_sequencer.clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (user, sequencer, runner)
}

/// Tests that the visible kernel hash does not update for each slot for the soft confirmations Kernel
/// The finalize hook hash should always match the most recent slot hash.
#[test]
fn visible_hash_soft_confirmations_kernel() {
    let (_, _, mut runner) = setup();

    let genesis_hash = *runner.state_root();

    let num_slots: u64 = config_value!("DEFERRED_SLOTS_COUNT") - 1;

    // We run `DEFERRED_SLOTS_COUNT` - 1 slots. The user and the finalize hash should not update
    last_state_root_closure(
        &mut |TestClosureArgs {
                  finalize_hook_hash,
                  current_slot_hash,
                  prev_slot_hash,
                  ..
              }| {
            assert_eq!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                genesis_hash.namespace_root(ProvableNamespace::User),
                "The user state root should not update until the visible state root updates"
            );

            assert_ne!(
                current_slot_hash.namespace_root(ProvableNamespace::Kernel),
                prev_slot_hash.namespace_root(ProvableNamespace::Kernel),
                "The kernel state root should update at every slot"
            );

            assert_eq!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                prev_slot_hash.namespace_root(ProvableNamespace::User),
                "The user state root should not update"
            );

            assert_eq!(
                finalize_hook_hash, genesis_hash,
                "The finalize hash and the genesis hash should always be the same"
            );
        },
        &mut runner,
        num_slots,
    );

    // We run 1 more slot. The user hash should update
    last_state_root_closure(
        &mut |TestClosureArgs {
                  current_slot_hash,
                  prev_slot_hash,
                  ..
              }| {
            assert_ne!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                prev_slot_hash.namespace_root(ProvableNamespace::User),
                "The user state root should update because the kernel slot hook updates"
            );

            assert_ne!(
                current_slot_hash.namespace_root(ProvableNamespace::Kernel),
                prev_slot_hash.namespace_root(ProvableNamespace::Kernel),
                "The kernel state root should update at every slot"
            );
        },
        &mut runner,
        1,
    );
}

#[test]
fn begin_slot_hash_soft_confirmations_kernel() {
    let (_, _, mut runner) = setup();

    let genesis_hash = *runner.state_root();

    let num_slots_before_first_rollup_block: u64 = config_value!("DEFERRED_SLOTS_COUNT") - 1;
    let state_root_delay_blocks: u64 = config_value!("STATE_ROOT_DELAY_BLOCKS");

    if num_slots_before_first_rollup_block <= 1 {
        panic!("DEFERRED_SLOTS_COUNT must be at least 3 for this test to function. If you're not using soft confirmations, you can safely ignore this failure. Otherwise, set your deferred slots count to at least 3.");
    }

    //  Get the post state roots of the first two slots.
    let mut first_slot_post_root = None;
    last_state_root_closure(
        &mut |TestClosureArgs {
                  current_slot_hash, ..
              }| {
            first_slot_post_root = Some(current_slot_hash);
        },
        &mut runner,
        1,
    );

    let mut second_slot_post_root = None;
    last_state_root_closure(
        &mut |TestClosureArgs {
                  current_slot_hash, ..
              }| {
            second_slot_post_root = Some(current_slot_hash);
        },
        &mut runner,
        1,
    );

    // Run more slots, stopping before the first rollup block is created..
    // You can verify that after this call total number of slots will be num_slots_before_first_rollup_block - 1
    runner.advance_slots(num_slots_before_first_rollup_block.saturating_sub(3) as usize);

    // Run one more slot, bring our total to exactly `num_slots_before_first_rollup_block`.
    // Since we're still before the first rollup block, the user space root should still match the genesis hash after this slot.
    let mut prev_finalize_hook_hash = None;
    last_state_root_closure(
        &mut |TestClosureArgs {
                  current_slot_hash,
                  finalize_hook_hash,
                  ..
              }| {
            assert_eq!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                genesis_hash.namespace_root(ProvableNamespace::User)
            );
            prev_finalize_hook_hash = Some(finalize_hook_hash);
        },
        &mut runner,
        1,
    );

    // Run a slot. This will create the first rollup block.
    let mut first_rollup_block_root = None;
    last_state_root_closure(
        &mut |TestClosureArgs {
                  begin_slot_hash,
                  current_slot_hash,
                  finalize_hook_hash,
                  ..
              }| {
            // This slot should still only have the genesis hash visible to it.
            assert_eq!(begin_slot_hash.unwrap(), genesis_hash);
            assert_eq!(
                begin_slot_hash.unwrap(),
                prev_finalize_hook_hash.unwrap(),
                "The begin slot hash should always match the previous finalize hook hash"
            );
            // Since we've created a rollup block, the output user state root should be different than the genesis hash
            assert_ne!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                genesis_hash.namespace_root(ProvableNamespace::User)
            );
            prev_finalize_hook_hash = Some(finalize_hook_hash);
            first_rollup_block_root = Some(current_slot_hash);
        },
        &mut runner,
        1,
    );
    let mut has_asserted_1 = false;
    let mut has_asserted_2 = false;

    // Run another slot. This will create the second rollup block.
    let mut second_rollup_block_root = None;
    last_state_root_closure(
        &mut |TestClosureArgs {
                  begin_slot_hash,
                  current_slot_hash,
                  finalize_hook_hash,
                  ..
              }| {
            // Since we've created a rollup block, the output user state root should be different than the previous rollup block
            assert_ne!(
                current_slot_hash.namespace_root(ProvableNamespace::User),
                first_rollup_block_root
                    .unwrap()
                    .namespace_root(ProvableNamespace::User)
            );
            assert_eq!(
                begin_slot_hash.unwrap(),
                prev_finalize_hook_hash.unwrap(),
                "The begin slot hash should always match the previous finalize hook hash"
            );
            second_rollup_block_root = Some(current_slot_hash);
            if state_root_delay_blocks == 0 {
                has_asserted_1 = true;
                assert_eq!(
                    begin_slot_hash
                        .unwrap()
                        .namespace_root(ProvableNamespace::User),
                    first_rollup_block_root
                        .unwrap()
                        .namespace_root(ProvableNamespace::User)
                );
                assert_eq!(
                    begin_slot_hash
                        .unwrap()
                        .namespace_root(ProvableNamespace::Kernel),
                    first_slot_post_root
                        .unwrap()
                        .namespace_root(ProvableNamespace::Kernel),
                    "Kernel roots didn't match. Found {}",
                    hex::encode(
                        begin_slot_hash
                            .unwrap()
                            .namespace_root(ProvableNamespace::Kernel)
                    )
                );
            }
            prev_finalize_hook_hash = Some(finalize_hook_hash);
        },
        &mut runner,
        1,
    );

    for prev_block_number in 2..=(state_root_delay_blocks + 2) {
        last_state_root_closure(
            &mut |TestClosureArgs {
                      begin_slot_hash,
                      finalize_hook_hash,
                      ..
                  }| {
                // Assert that the begin slot hash is what we expected
                let block_that_should_be_visible =
                    prev_block_number.saturating_sub(state_root_delay_blocks);
                assert_eq!(
                    begin_slot_hash.unwrap(),
                    prev_finalize_hook_hash.unwrap(),
                    "The begin slot hash should always match the previous finalize hook hash"
                );
                if block_that_should_be_visible == 0 {
                    assert_eq!(begin_slot_hash.unwrap(), genesis_hash);
                } else if block_that_should_be_visible == 1 {
                    assert_eq!(
                        begin_slot_hash
                            .unwrap()
                            .namespace_root(ProvableNamespace::User),
                        first_rollup_block_root
                            .unwrap()
                            .namespace_root(ProvableNamespace::User)
                    );
                    assert_eq!(
                        begin_slot_hash
                            .unwrap()
                            .namespace_root(ProvableNamespace::Kernel),
                        first_slot_post_root
                            .unwrap()
                            .namespace_root(ProvableNamespace::Kernel),
                        "Kernel roots didn't match. Found {}",
                        hex::encode(
                            begin_slot_hash
                                .unwrap()
                                .namespace_root(ProvableNamespace::Kernel)
                        )
                    );
                    has_asserted_1 = true;
                } else {
                    assert_eq!(
                        begin_slot_hash
                            .unwrap()
                            .namespace_root(ProvableNamespace::User),
                        second_rollup_block_root
                            .unwrap()
                            .namespace_root(ProvableNamespace::User)
                    );
                    assert_eq!(
                        begin_slot_hash
                            .unwrap()
                            .namespace_root(ProvableNamespace::Kernel),
                        second_slot_post_root
                            .unwrap()
                            .namespace_root(ProvableNamespace::Kernel),
                        "Kernel roots didn't match. Found {}",
                        hex::encode(
                            begin_slot_hash
                                .unwrap()
                                .namespace_root(ProvableNamespace::Kernel)
                        )
                    );
                    has_asserted_2 = true;
                }
                prev_finalize_hook_hash = Some(finalize_hook_hash);
            },
            &mut runner,
            1,
        );
    }
    assert!(
        has_asserted_1,
        "We should have asserted the first rollup block"
    );
    assert!(
        has_asserted_2,
        "We should have asserted the second rollup block"
    );
}
