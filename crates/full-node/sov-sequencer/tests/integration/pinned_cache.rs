use crate::preferred_end_to_end::DaLayerWithSubscription;
use crate::utils::tempdir_inside_codebase_dir;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types as api_types;
use sov_db::storage_manager::NomtStorageManager;
use sov_mock_da::BlockProducingConfig;
use sov_mock_da::MockHash;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::CryptoSpec;
use sov_modules_api::RawTx;
use sov_modules_api::Spec;
use sov_modules_api::{DispatchCall, HexHash, HexString};
use sov_modules_stf_blueprint::Runtime;
use sov_sequencer::SequencerKindConfig;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::pinned_cache::PinnedCache;
use sov_state::DefaultStorageSpec;
use sov_test_modules::pinned_cache::CallMessage as PinnedCacheCallMessage;
use sov_test_modules::pinned_cache::PinnedCacheTester;
use sov_test_modules::pinned_cache::ValueRange;
use sov_test_utils::generate_optimistic_runtime_with_kernel;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::MockDaSpec;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::TestNomtSpec as TestSpec;
use sov_test_utils::TestUser;
use sov_test_utils::{default_test_signed_transaction, TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS};
use tokio_stream::StreamExt;

type TestNomtBlueprint = RtAgnosticBlueprint<
    TestSpec,
    TestRuntime<TestSpec>,
    NomtStorageManager<
        MockDaSpec,
        <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher,
        NomtProverStorage<
            DefaultStorageSpec<<<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
            MockHash,
        >,
    >,
>;

const PINNED_ADDRESS: HexHash = HexString([1u8; 32]);

// generate_optimistic_runtime_with_kernel!(
//     TestRuntime <=
//     kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
//     modules: [pinned_cache_tester: PinnedCacheTester<S>],
//     populate_pinned_cache_fn: |_storage: &S::Storage| {
//         Some(PinnedCache::default())
//     }
// );

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [pinned_cache_tester: PinnedCacheTester<S>],
    populate_pinned_cache_fn: |storage: &S::Storage| {
        let mut cache = PinnedCache::default();
        let bucket_id = PinnedCacheTester::<S>::default().get_bucket_id(&PINNED_ADDRESS);
        cache.try_load_bucket_if_absent(bucket_id, storage, 1000).unwrap();
        Some(cache)
    }
);

async fn create_test_nomt_rollup() -> (TestRollup<TestNomtBlueprint>, TestUser<TestSpec>) {
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
        c.max_concurrent_blobs = 64;
        if let SequencerKindConfig::Preferred(ref mut config) = &mut c.sequencer_config {
            config.num_cache_warmup_workers = 0;
            config.batch_execution_time_limit_millis = 6000;
        }
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da()
    .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave);

    (builder.start().await.unwrap(), admin)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_basic_pinning() {
    // sov_test_utils::initialize_logging();

    let nb_of_blocks = 5;
    let (test_rollup, user) = create_test_nomt_rollup().await;

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;

    // Send a transaction which will block the sequencer for a while. This gives us time to send other txs with different priorities and ensure
    // that the priority tiebreaker is working.
    let client = test_rollup.api_client().clone();
    let tx = tx_read_pinned_cache(
        &user.private_key,
        0,
        PINNED_ADDRESS,
        Some(ValueRange {
            indices: 0..1,
            value: 0,
        }),
        Some(0),
    ); // A read from the pinned bucket should not fall through to storage.

    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    let mut off_by_one_address = PINNED_ADDRESS;
    off_by_one_address.0[31] += 1;
    let tx = tx_read_pinned_cache(
        &user.private_key,
        0,
        off_by_one_address,
        Some(ValueRange {
            indices: 0..1,
            value: 0,
        }),
        Some(1),
    ); // Should fall to storage because this address is not pinned

    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    test_rollup.shutdown().await.unwrap();
}

/// A basic test of ram pinning. Write to some slots and not others. Check that no matter what slot we read, we get the correct value
/// and don't touch storage.
#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_basic_pinning_with_writes() {
    sov_test_utils::initialize_logging();

    let nb_of_blocks = 5;
    let (test_rollup, user) = create_test_nomt_rollup().await;

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;
    let client = test_rollup.api_client().clone();
    let mut off_by_one_address = PINNED_ADDRESS;
    off_by_one_address.0[31] += 1;

    for i in 0..8 {
        let tx = tx_write_pinned_cache(
            &user.private_key,
            i as u64,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        for j in 0u32..=i {
            let expected_value = if j <= i { j } else { 0 };
            // Read from the pinned address. We should never touch storage.
            let tx = tx_read_pinned_cache(
                &user.private_key,
                i as u64,
                PINNED_ADDRESS,
                Some(ValueRange {
                    indices: j..j + 1,
                    value: expected_value,
                }),
                Some(0),
            );
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .unwrap();
        }

        test_rollup.force_close_batch().await.unwrap();
        da_layer.produce_and_wait_for_n_slots(1).await;
    }
    da_layer.produce_and_wait_for_n_slots(4).await;
    test_rollup.shutdown().await.unwrap();
}

/// Test ram pinning with a non-pinned address
#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_basic_pinning_with_writes_not_cached_address() {
    let nb_of_blocks = 5;
    let (test_rollup, user) = create_test_nomt_rollup().await;

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;
    let client = test_rollup.api_client().clone();
    let mut off_by_one_address = PINNED_ADDRESS;
    off_by_one_address.0[31] += 1;

    for i in 0..8 {
        // On each iteration, write to both the pinned and non-pinned addresses.
        let tx = tx_write_pinned_cache(
            &user.private_key,
            i as u64,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
        ); // A read from the pinned bucket should not fall through to storage.
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        let tx = tx_write_pinned_cache(
            &user.private_key,
            i as u64,
            off_by_one_address,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
        ); // A read from the pinned bucket should not fall through to storage.
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();

        for j in 0u32..=i {
            let expected_value = if j <= i { j } else { 0 };
            // Read from the pinned address. We should never touch storage.
            let tx = tx_read_pinned_cache(
                &user.private_key,
                i as u64,
                PINNED_ADDRESS,
                Some(ValueRange {
                    indices: j..j + 1,
                    value: expected_value,
                }),
                Some(0),
            );
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .unwrap();

            // Read from the non-pinned address. We should touch storage if needed
            let should_touch_storage = j != i;
            let tx = tx_read_pinned_cache(
                &user.private_key,
                i as u64,
                off_by_one_address,
                Some(ValueRange {
                    indices: j..j + 1,
                    value: expected_value,
                }),
                Some(should_touch_storage as u64),
            );
            client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .unwrap();
        }

        test_rollup.force_close_batch().await.unwrap();
        da_layer.produce_and_wait_for_n_slots(1).await;
    }
}

/// Test that pinning still works after the sequencer has exited recovery.
///
/// This test works by...
/// - Sending a transaction to test initial setup
/// - Sending the sequencer into recovery by producing a lot of blocks
/// - Letting the sequencer exit recovery
/// - Sending a test transaction to read the pinned cache. Ensure that it didn't touch storage.
/// - Letting that transaction go through to the full node to ensure that works as well.
#[tokio::test(flavor = "multi_thread")]
async fn test_pinning_after_recovery() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");
    // sov_test_utils::initialize_logging();
    let (test_rollup, admin) = create_test_nomt_rollup().await;

    let client = test_rollup.api_client().clone();
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;

    // Finalise some blocks
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    println!("1");

    // Sanity check tx that the rollup works, and send the tx all the way through to DA.
    // Set a value in the pinned cache.
    let tx = tx_write_pinned_cache(
        &admin.private_key,
        0,
        PINNED_ADDRESS,
        Some(ValueRange {
            indices: 0..1,
            value: 1,
        }),
    );
    client.send_raw_tx_to_sequencer(&tx).await.unwrap();
    test_rollup.force_close_batch().await.unwrap();
    da_layer.produce_and_wait_for_n_slots(1).await;
    println!("2");

    // Pause sequencer update_state and run some blocks so deferred_slots_count is reached
    test_rollup.pause_preferred_batches().await;
    tracing::info!("Preferred sequencer batch production paused.");
    tracing::info!(
        "Producing subsequent DA blocks while sequencer is paused, to exceed deferred_slots_count"
    );
    // This can be lower than DEFERRED_SLOTS_COUNT because the sequencer takes into account a)
    // possible node lag and b) a 90% threshold.
    test_rollup.tenderly_produce_blocks(40).await.unwrap();
    // Make sure the DA has synced everything
    test_rollup.wait_for_node_synced().await.unwrap();
    println!("2");

    tracing::info!("Resuming preferred sequencer batch production.");
    test_rollup.resume_preferred_batches().await;
    println!("3");
    // Normally on the next state update, the sequencer should always enter recovery.
    // However for some reason this was flaky.
    test_rollup.tenderly_produce_blocks(1).await.unwrap();
    let max_wait = 1000;
    let mut i = 0;
    let reasonable_time_for_rollup =
        std::time::Duration::from_millis(TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS);
    while test_rollup.is_sequencer_ready().await {
        // For some reason DA subscriptions are still broken at this point; if we use produce_and_wait_for_n_slots, the test will hang.
        // This is unrelated to the feature under test, so I've left it for now. Anyone reading this should feel free to change the test.
        test_rollup.tenderly_produce_blocks(1).await.unwrap();
        i += 1;
        if i > max_wait {
            panic!("sequencer never became ready in {max_wait} blocks");
        }
        tokio::time::sleep(reasonable_time_for_rollup).await;
    }
    test_rollup.wait_for_node_synced().await.unwrap();
    println!("4");

    i = 0;
    while !test_rollup.is_sequencer_ready().await {
        // For some reason DA subscriptions are still broken at this point; if we use produce_and_wait_for_n_slots, the test will hang.
        // This is unrelated to the feature under test, so I've left it for now. Anyone reading this should feel free to change the test.
        test_rollup.tenderly_produce_blocks(1).await.unwrap();
        if i > max_wait {
            panic!("sequencer never became ready in {max_wait} blocks");
        }
        i += 1;
        tokio::time::sleep(reasonable_time_for_rollup).await;
    }
    println!("5");

    // Read the value that should be in pinned cache. We should get the correct value (1) and not touch storage.
    let tx2 = tx_read_pinned_cache(
        &admin.private_key,
        0,
        PINNED_ADDRESS,
        Some(ValueRange {
            indices: 0..1,
            value: 1,
        }),
        Some(0),
    );
    client.send_raw_tx_to_sequencer(&tx2).await.unwrap();
    println!("6");

    test_rollup.force_close_batch().await.unwrap();
    // For some reason DA subscriptions are still broken at this point; if we use produce_and_wait_for_n_slots, the test will hang.
    // So we just produce blocks and sleep
    test_rollup.tenderly_produce_blocks(3).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), test_rollup.shutdown())
        .await
        .unwrap()
        .unwrap();
}

/// Ensures that RAM pinning still works after a total resync.
#[tokio::test(flavor = "multi_thread")]
async fn test_pinned_cache_after_total_resync() {
    let (test_rollup, admin) = create_test_nomt_rollup().await;
    // Finalise some blocks
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;
    let client = test_rollup.api_client().clone();

    // Send some initial transactions to write values
    for i in 0..40 {
        let tx = tx_write_pinned_cache(
            &admin.private_key,
            i as u64,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();
        da_layer.produce_and_wait_for_n_slots(1).await;
    }

    // Shutdown and wipe the rollup (except the preferred sequencer DB and DA)
    let builder = test_rollup.shutdown().await.unwrap();
    let rollup_storage_path = builder.storage_path();
    // Next, delete everything except the preferred sequencer DB. Resync again to verify that this
    // doesn't interfere
    for path in [
        "user_nomt_db",
        "state-db",
        "archival-state-db",
        "kernel_nomt_db",
        "accessory",
        "ledger",
        "blob_sender",
    ] {
        std::fs::remove_dir_all(rollup_storage_path.path().join(path)).unwrap();
    }

    // Restart the rollup and wait for it to resync
    let test_rollup = builder.start().await.unwrap();
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    // Wait until we've mostly resynced to ensure there's no flakiness from the sequencer readiness endpoint.
    loop {
        let slot =
            tokio::time::timeout(std::time::Duration::from_secs(10), slot_subscription.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        if slot.batch_range.end == 30 {
            break;
        }
    }
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    da_layer.produce_and_wait_for_n_slots(5).await;

    // Send transactions to read the pinned cache. We should get the correct values (i) and not touch storage.
    let client = test_rollup.api_client().clone();
    for i in 0..40 {
        let tx = tx_read_pinned_cache(
            &admin.private_key,
            (i as u64) + 40,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
            Some(0),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        if i % 5 == 0 {
            test_rollup.force_close_batch().await.unwrap();
            da_layer.produce_and_wait_for_n_slots(1).await;
        }
    }

    da_layer.produce_and_wait_for_n_slots(5).await;
    test_rollup.shutdown().await.unwrap();
}

/// Ensures that RAM pinning works again after the node falls out of sync
#[tokio::test(flavor = "multi_thread")]
async fn test_pinned_cache_after_fast_resync() {
    sov_test_utils::initialize_logging();
    let (test_rollup, admin) = create_test_nomt_rollup().await;
    // Finalise some blocks
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;
    let client = test_rollup.api_client().clone();

    for i in 0..40 {
        let tx = tx_write_pinned_cache(
            &admin.private_key,
            i as u64,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();
        da_layer.produce_and_wait_for_n_slots(1).await;
    }

    // Produce blocks in rapid succession to unsync the node
    for _ in 0..20 {
        da_layer.produce_block().await.unwrap();
    }
    test_rollup.wait_for_sequencer_not_ready().await.unwrap(); // verify that it becomes unready, then wait for it to come back up.
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Send transactions to read the pinned cache. We should get the correct values (i) and not touch storage.
    let client = test_rollup.api_client().clone();
    for i in 0..40 {
        let tx = tx_read_pinned_cache(
            &admin.private_key,
            (i as u64) + 40,
            PINNED_ADDRESS,
            Some(ValueRange {
                indices: i..i + 1,
                value: i,
            }),
            Some(0),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        if i % 5 == 0 {
            test_rollup.force_close_batch().await.unwrap();
            da_layer.produce_and_wait_for_n_slots(1).await;
        }
    }

    test_rollup.shutdown().await.unwrap();
}

fn tx_read_pinned_cache(
    key: &Ed25519PrivateKey,
    generation: u64,
    address: HexHash,
    read_indexes: Option<ValueRange>,
    expected_storage_accesses: Option<u64>,
) -> RawTx {
    tx_check_pinned_cache(
        key,
        generation,
        address,
        read_indexes,
        None,
        expected_storage_accesses,
    )
}

fn tx_write_pinned_cache(
    key: &Ed25519PrivateKey,
    generation: u64,
    address: HexHash,
    write_indexes: Option<ValueRange>,
) -> RawTx {
    tx_check_pinned_cache(key, generation, address, None, write_indexes, None)
}

fn tx_check_pinned_cache(
    key: &Ed25519PrivateKey,
    generation: u64,
    address: HexHash,
    read_indexes: Option<ValueRange>,
    write_indexes: Option<ValueRange>,
    expected_storage_accesses: Option<u64>,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::PinnedCacheTester(
        PinnedCacheCallMessage::TestCacheAccesses {
            address,
            read_indexes,
            write_indexes,
            expected_storage_accesses,
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
