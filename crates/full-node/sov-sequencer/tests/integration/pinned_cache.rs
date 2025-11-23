use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::{types as api_types};
use sov_db::storage_manager::NomtStorageManager;
use sov_mock_da::MockHash;
use sov_modules_api::CryptoSpec;
use sov_modules_api::Spec;
use sov_test_modules::pinned_cache::ValueRange;
use sov_test_utils::default_test_signed_transaction;
use sov_test_utils::MockDaSpec;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_sequencer::SequencerKindConfig;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_test_utils::generate_optimistic_runtime_with_kernel;
use sov_state::pinned_cache::PinnedCache;
use sov_test_modules::pinned_cache::PinnedCacheTester;
use sov_modules_stf_blueprint::Runtime;
use sov_modules_api::RawTx;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::RtAgnosticBlueprint;
use sov_modules_api::{DispatchCall, HexHash, HexString};
use sov_test_utils::TEST_MAX_CONCURRENT_BLOBS;
use crate::preferred_end_to_end::DaLayerWithSubscription;
use sov_test_utils::runtime::GenesisParams;
use crate::utils::tempdir_inside_codebase_dir;
use sov_test_utils::test_rollup::TestRollup;
use sov_mock_da::BlockProducingConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::TestUser;
use sov_test_utils::TestNomtSpec as TestSpec;
use sov_test_modules::pinned_cache::CallMessage as PinnedCacheCallMessage;

type TestNomtBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>, NomtStorageManager<MockDaSpec,  <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher,
	NomtProverStorage<
		DefaultStorageSpec<<<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
		MockHash,
	>,>>;


const PINNED_ADDRESS: HexHash = HexString([1u8;32]);


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

	let seq_da_address = genesis_params.runtime.sequencer_registry.sequencer_config.seq_da_address;

    let dir = tempdir_inside_codebase_dir();

	let builder = RollupBuilder::<TestNomtBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
		if let SequencerKindConfig::Preferred(ref mut config) = &mut c.sequencer_config {
			config.num_cache_warmup_workers = 0;
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
	let tx = tx_read_pinned_cache(&user.private_key, 0, PINNED_ADDRESS, Some(ValueRange { indices: 0..1, value: 0 }),  Some(0)); // A read from the pinned bucket should not fall through to storage.

	client
	.accept_tx(&api_types::AcceptTxBody {
		body: BASE64_STANDARD.encode(&tx),
	})
	.await
	.unwrap();

	let mut off_by_one_address = PINNED_ADDRESS.clone();
	off_by_one_address.0[31] += 1;
	let tx = tx_read_pinned_cache(&user.private_key, 0, off_by_one_address, Some(ValueRange { indices: 0..1, value: 0 }), Some(1)); // Should fall to storage because this address is not pinned

	client
	.accept_tx(&api_types::AcceptTxBody {
		body: BASE64_STANDARD.encode(&tx),
	})
	.await
	.unwrap();

	test_rollup.shutdown().await.unwrap();
}


#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_basic_pinning_with_writes() {
	sov_test_utils::initialize_logging();

	let nb_of_blocks = 5;
	let (test_rollup, user) = create_test_nomt_rollup().await;

	let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;
	let client = test_rollup.api_client().clone();
	let mut off_by_one_address = PINNED_ADDRESS.clone();
	off_by_one_address.0[31] += 1;

	for i in 0..8 {
		let tx = tx_write_pinned_cache(&user.private_key, i as u64, PINNED_ADDRESS, Some(ValueRange { indices: i..i+1, value: i as u32})); // A read from the pinned bucket should not fall through to storage.
			client
			.accept_tx(&api_types::AcceptTxBody {
				body: BASE64_STANDARD.encode(&tx),
			})
			.await
			.unwrap();

		for j in 0u32..=i {
			let expected_value = if j <= i { j as u32 } else { 0 };
			// Read from the pinned address. We should never touch storage.
			let tx = tx_read_pinned_cache(&user.private_key, i as u64, PINNED_ADDRESS, Some(ValueRange { indices: j..j+1, value: expected_value as u32}),  Some(0)); // A read from the pinned bucket should not fall through to storage.
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


#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_basic_pinning_with_writes_not_cached_address() {
	// sov_test_utils::initialize_logging();

	let nb_of_blocks = 5;
	let (test_rollup, user) = create_test_nomt_rollup().await;

	let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;
	let client = test_rollup.api_client().clone();
	let mut off_by_one_address = PINNED_ADDRESS.clone();
	off_by_one_address.0[31] += 1;

	for i in 0..8 {
		println!("Writing to slot {}", i);
		let tx = tx_write_pinned_cache(&user.private_key, i as u64, PINNED_ADDRESS, Some(ValueRange { indices: i..i+1, value: i as u32})); // A read from the pinned bucket should not fall through to storage.
			client
			.accept_tx(&api_types::AcceptTxBody {
				body: BASE64_STANDARD.encode(&tx),
			})
			.await
			.unwrap();

		let tx = tx_write_pinned_cache(&user.private_key, i as u64, off_by_one_address, Some(ValueRange { indices: i..i+1, value: i as u32})); // A read from the pinned bucket should not fall through to storage.
			client
			.accept_tx(&api_types::AcceptTxBody {
				body: BASE64_STANDARD.encode(&tx),
			})
			.await
			.unwrap();

		for j in 0u32..=i {
			let expected_value = if j <= i { j as u32 } else { 0 };
			println!("Reading from slot {}. Expected value: {}", j, expected_value);
			// Read from the pinned address. We should never touch storage.
			let tx = tx_read_pinned_cache(&user.private_key, i as u64, PINNED_ADDRESS, Some(ValueRange { indices: j..j+1, value: expected_value as u32}),  Some(0)); // A read from the pinned bucket should not fall through to storage.
			client
			.accept_tx(&api_types::AcceptTxBody {
				body: BASE64_STANDARD.encode(&tx),
			})
			.await
			.unwrap();

			// Read from the pinned address. We should never touch storage
			let should_touch_storage = j != i;
			let tx = tx_read_pinned_cache(&user.private_key, i as u64, off_by_one_address, Some(ValueRange { indices: j..j+1, value: expected_value as u32}),  Some(should_touch_storage as u64)); // A read from the pinned bucket should not fall through to storage.
			client
			.accept_tx(&api_types::AcceptTxBody {
				body: BASE64_STANDARD.encode(&tx),
			})
			.await
			.unwrap();

		}
		

		
		test_rollup.force_close_batch().await.unwrap();
		// da_layer.produce_and_wait_for_n_slots(1).await;
	}

}



fn tx_read_pinned_cache(key: &Ed25519PrivateKey, generation: u64, address: HexHash, read_indexes: Option<ValueRange>, expected_storage_accesses: Option<u64>) -> RawTx {
	tx_check_pinned_cache(key, generation, address, read_indexes, None, expected_storage_accesses)
}

	
fn tx_write_pinned_cache(key: &Ed25519PrivateKey, generation: u64, address: HexHash, write_indexes: Option<ValueRange>) -> RawTx {
	tx_check_pinned_cache(key, generation, address, None, write_indexes, None)
}



fn tx_check_pinned_cache(key: &Ed25519PrivateKey, generation: u64, address: HexHash, read_indexes: Option<ValueRange>, write_indexes: Option<ValueRange>, expected_storage_accesses: Option<u64>) -> RawTx {
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
