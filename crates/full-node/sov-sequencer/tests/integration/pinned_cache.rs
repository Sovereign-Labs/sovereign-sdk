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



generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [pinned_cache_tester: PinnedCacheTester<S>],
    populate_pinned_cache_fn: |_storage: &S::Storage| {
        Some(PinnedCache::default())
    }
);


// generate_optimistic_runtime_with_kernel!(
//     TestRuntimeWithPinnedCache <=
//     kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
//     modules: [pinned_cache_tester: PinnedCacheTester<S>],
//     populate_pinned_cache_fn: |_storage: &S::Storage| {

//     }
// );


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
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da()
    .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave);

    (builder.start().await.unwrap(), admin)
}


#[tokio::test(flavor = "multi_thread")]
async fn test_nomt_rollup() {
	let nb_of_blocks = 5;
	let (test_rollup, user) = create_test_nomt_rollup().await;

	let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(nb_of_blocks).await;

    // Send a transaction which will block the sequencer for a while. This gives us time to send other txs with different priorities and ensure
    // that the priority tiebreaker is working.
    let client = test_rollup.api_client().clone();
    // let tx = tx_set_value_and_sleep(&admin.private_key, 0, 1000, 5000);
	let tx = tx_check_pinned_cache(&user.private_key, 0, HexString([0u8;32]), Some(ValueRange { indices: 0..1, value: 0 }), None, Some(1));

	client
	.accept_tx(&api_types::AcceptTxBody {
		body: BASE64_STANDARD.encode(&tx),
	})
	.await
	.unwrap();
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
