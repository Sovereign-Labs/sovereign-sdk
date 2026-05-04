use std::collections::BTreeMap;

use crate::preferred_end_to_end::DaLayerWithSubscription;
use crate::utils::tempdir_inside_codebase_dir;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types as api_types;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::RawTx;
use sov_modules_api::{DispatchCall, HexHash, HexString};
use sov_modules_stf_blueprint::Runtime;
use sov_rest_utils::PaginatedResponse;
use sov_sequencer::SequencerKindConfig;
use sov_test_modules::pinned_cache::CallMessage;
use sov_test_modules::pinned_cache::StateKey;
use sov_test_modules::pinned_cache::StateMapTester;
use sov_test_utils::default_test_signed_transaction;
use sov_test_utils::generate_optimistic_runtime_with_kernel;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::TestSpec;
use sov_test_utils::TestStorageManager;
use sov_test_utils::TestUser;

type TestNomtBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>, TestStorageManager>;

const STATE_MAP_ADDRESS: HexHash = HexString([1u8; 32]);

fn tx_modify_state_map(
    key: &Ed25519PrivateKey,
    generation: u64,
    address: HexHash,
    index: u32,
    new_value: Option<u32>,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::StateMapTester(
        CallMessage::ModifyStateMap {
            address,
            index,
            new_value,
        },
    );

    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        &msg,
        generation,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [state_map_tester: StateMapTester<S>],
);

async fn create_test_nomt_rollup_with_long_finalization(
) -> (TestRollup<TestNomtBlueprint>, TestUser<TestSpec>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            (),
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let seq_da_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;

    let dir = tempdir_inside_codebase_dir();

    let builder = RollupBuilder::<TestNomtBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
        c.max_concurrent_batch_blobs = 64;
        if let SequencerKindConfig::Preferred(ref mut config) = &mut c.sequencer_config {
            config.num_cache_warmup_workers = 0;
            config.batch_execution_time_limit_millis = 6000;
            config.ideal_lag_behind_finalized_slot = 10;
        }
    })
    .set_da_config(|c| {
        c.sender_address = seq_da_address;
        c.finalization_blocks = 10;
    })
    .set_persistent_da()
    .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave);

    (builder.start().await.unwrap(), admin)
}

async fn send_value_and_update_values(
    test_rollup: &TestRollup<TestNomtBlueprint>,
    user: &TestUser<TestSpec>,
    key: (HexHash, u32),
    new_value: Option<u32>,
    values: &mut BTreeMap<StateKey, u32>,
    iter: u32,
) {
    let tx = tx_modify_state_map(&user.private_key, iter as u64, key.0, key.1, new_value);
    test_rollup
        .api_client()
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();
    if let Some(new_value) = new_value {
        values.insert(
            StateKey {
                address: key.0,
                index: key.1,
            },
            new_value,
        );
    } else {
        values.remove(&StateKey {
            address: key.0,
            index: key.1,
        });
    }
}

// Generate a permutation of 0..255
fn permutation_key(n: u32) -> u32 {
    // 37 is coprime with 256, so this walks a full permutation of 0..255.
    (n * 37 + 11) % 256
}

async fn do_step(
    da_layer: &mut DaLayerWithSubscription,
    test_rollup: &TestRollup<TestNomtBlueprint>,
    user: &TestUser<TestSpec>,
    key: (HexHash, u32),
    new_value: Option<u32>,
    values: &mut BTreeMap<StateKey, u32>,
    iter: u32,
) {
    // Step 1: Send a tx to update the state map.
    send_value_and_update_values(test_rollup, user, key, new_value, values, iter).await;

    // Step 2: Produce a new block, maybe notifying the sequencer
    //
    // This if/else is used to exercise the `uncommited changes` path in the sequencer.
    // For a few blocks (from 20-25), we skip state updates from the node but force close batches. This
    // causes the sequencer to store multiple blocks worth of updates in its uncommitted changes.
    //
    // The rest of the time, we produce a new block and let the state update flow to the sequencer normally.
    if (20..25).contains(&iter) {
        if iter == 20 {
            test_rollup.pause_preferred_batches().await;
        }
        da_layer.produce_block().await.unwrap();
        test_rollup.force_close_batch().await.unwrap();
    } else {
        if iter == 25 {
            test_rollup.resume_preferred_batches().await;
        }
        // The normal path:
        da_layer.produce_and_wait_for_slot().await;
    }

    // Step 3: Assert that the state map values returned from the REST API match the local BTreeMap of expected items.
    fetch_and_assert_state_map_values(test_rollup, values).await;
}

/// Test that state map iteration yields the correct values.
///
/// The test works by running 100 slots with one operation at each slot. 1/3 of the ops are deletes,
/// and the remaining 2/3 are inserts. We insert at keys from 0 to 255 in a pseudo-random order, and we
/// always delete one of the two keys that was just inserted (for simplicity).
#[tokio::test(flavor = "multi_thread")]
async fn test_state_map_iteration() {
    let nb_of_blocks = 15;
    let (test_rollup, user) = create_test_nomt_rollup_with_long_finalization().await;

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;

    let mut values = BTreeMap::new();

    let mut insert_idx = 0;
    // We run 100 slots and run a different operation at each slot. This should all kinds of state changes,
    // including uncomitted changes, finalization, and the live state map. At each step, we assert that the result
    // returned from the REST API exactly matches the local BTreeMap of expected items.
    for i in 0..100 {
        let address = STATE_MAP_ADDRESS;

        // This code block generates a permutation of 0..255 and uses that to build a sequence of insertions and deletions in pseduo-random order.
        // We do 2 insertions and 1 deletion per group of 3 operations. This lets us verify that the keys are correctly returned in order
        // even when the insertions are out of order and deletions are mixed in.
        let (index, value) = match i % 3 {
            0 | 1 => {
                let k = permutation_key(insert_idx);
                insert_idx += 1;
                (k, Some(i))
            }
            2 => {
                // Delete the first key from this 3-op group.
                // For group g, inserts were key(2g), key(2g + 1).
                let group = i / 3;
                let k = permutation_key(2 * group);
                (k, None)
                // delete k
            }
            _ => unreachable!(),
        };
        do_step(
            &mut da_layer,
            &test_rollup,
            &user,
            (address, index),
            value,
            &mut values,
            i,
        )
        .await;
    }
    test_rollup.shutdown().await.unwrap();
}

async fn fetch_and_assert_state_map_values(
    test_rollup: &TestRollup<TestNomtBlueprint>,
    values: &BTreeMap<StateKey, u32>,
) {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let query_string = if let Some(cursor) = cursor {
            format!("?page=next&page%5Bcursor%5D={cursor}&page%5Bsize%5D=25")
        } else {
            String::new()
        };
        let url = format!("/modules/state-map-tester/state/values/items{query_string}");

        let response = test_rollup
            .client
            .query_rest_endpoint::<PaginatedResponse<StateItemContents<StateKey, u32>, String>>(
                &url,
            )
            .await
            .unwrap();
        items.extend(response.items);
        cursor = response.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(items.len(), values.len());
    for (item, value) in items.iter().zip(values.iter()) {
        assert_eq!(&item.key, value.0);
        assert_eq!(&item.value, value.1);
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StateItemContents<K, V> {
    pub key: K,
    pub value: V,
}
