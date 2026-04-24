use std::sync::Arc;

use sov_address::MultiAddressEvm;
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::macros::config_value;
use sov_modules_stf_blueprint::GenesisParams;
use sov_sequencer::{SeqConfigExtension, SequencerKindConfig, SovRateLimiterConfig};
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath, TestRollup};
use tempfile::TempDir;

use crate::common::genesis::{
    build_genesis, default_genesis, paymaster_selective_genesis, paymaster_with_payer_genesis,
    GenesisOptions, TestGenesis,
};
use crate::runtime::EvmBlueprint;

/// Builds a `TestRollup<EvmBlueprint>` from a pre-built [`TestGenesis`].
async fn start_node_with_genesis(
    test_genesis: TestGenesis,
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
    rate_limiter: Option<SovRateLimiterConfig<MultiAddressEvm>>,
    ideal_lag: u64,
) -> TestRollup<EvmBlueprint> {
    let dir = TempDir::new().expect("create tempdir");
    let TestGenesis {
        genesis,
        seq_da_address,
        ..
    } = test_genesis;

    // Shared DA layer so blobs submitted by the sequencer are visible to the block producer.
    let da_layer = Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(finalization_blocks)
            .await
            .expect("create in-memory DA layer"),
    ));

    let mut builder = RollupBuilder::<EvmBlueprint>::new(
        GenesisSource::CustomParams(GenesisParams { runtime: genesis }),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        finalization_blocks,
    );

    if let Some(rl) = rate_limiter {
        builder = builder.with_rate_limiter(Some(rl));
    }

    builder
        .set_config(|c| {
            c.storage = StoragePath::Tmp(dir.into());
            c.max_concurrent_blobs = 65536;
            c.rollup_prover_config = RollupProverConfig::Disabled;
            c.aggregated_proof_block_jump = 5;
            c.max_infos_in_db = 30;
            c.max_channel_size = 20;
            c.trusted_proxies = vec![std::net::Ipv4Addr::LOCALHOST.into()];
            c.extension = extension;
            if let SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
                seq.ideal_lag_behind_finalized_slot = ideal_lag;
            }
        })
        .set_da_config(|c| {
            c.sender_address = seq_da_address;
            c.da_layer = Some(da_layer.clone());
        })
        .start()
        .await
        .expect("start test rollup")
}

pub async fn setup_test_rollup(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<EvmBlueprint> {
    setup_test_rollup_with_ideal_lag(finalization_blocks, extension, 3).await
}

/// Sets up a test rollup with a custom rate limiter.
pub async fn setup_test_rollup_with_rate_limiter(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    rate_limiter: SovRateLimiterConfig<MultiAddressEvm>,
) -> TestRollup<EvmBlueprint> {
    start_node_with_genesis(
        default_genesis(),
        finalization_blocks,
        Some(extension),
        Some(rate_limiter),
        3,
    )
    .await
}

/// Like `setup_test_rollup`, but also returns the admin private key used by the EVM genesis.
pub async fn setup_test_rollup_with_admin_key(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (
    TestRollup<EvmBlueprint>,
    <<crate::runtime::EvmTestSpec as sov_modules_api::Spec>::CryptoSpec as sov_modules_api::CryptoSpec>::PrivateKey,
){
    let test_genesis = default_genesis();
    let admin_private_key = test_genesis.admin_private_key.clone();
    let rollup =
        start_node_with_genesis(test_genesis, finalization_blocks, Some(extension), None, 3).await;
    (rollup, admin_private_key)
}

pub async fn setup_test_rollup_with_ideal_lag(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    ideal_lag: u64,
) -> TestRollup<EvmBlueprint> {
    start_node_with_genesis(
        default_genesis(),
        finalization_blocks,
        Some(extension),
        None,
        ideal_lag,
    )
    .await
}

pub async fn setup_test_rollup_with_paymaster(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<EvmBlueprint> {
    let (rollup, _user) =
        setup_test_rollup_with_paymaster_user(finalization_blocks, extension).await;
    rollup
}

/// Like [`setup_test_rollup_with_paymaster`] but also returns the registered paymaster
/// (sov user) so tests can check its bank balance.
pub async fn setup_test_rollup_with_paymaster_user(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (
    TestRollup<EvmBlueprint>,
    sov_test_utils::TestUser<crate::runtime::EvmTestSpec>,
) {
    let test_genesis = paymaster_with_payer_genesis();
    let paymaster_user = test_genesis
        .paymaster
        .clone()
        .expect("paymaster_with_payer_genesis registers a paymaster");
    let rollup =
        start_node_with_genesis(test_genesis, finalization_blocks, Some(extension), None, 1).await;
    (rollup, paymaster_user)
}

/// Sets up a test rollup with a contract creation allowlist policy.
pub async fn setup_test_rollup_with_allowlist(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    allowlist: &[alloy_primitives::Address],
) -> TestRollup<EvmBlueprint> {
    let policy = sov_evm::ContractCreationPolicy::Allowlist(allowlist.iter().copied().collect());
    let test_genesis = build_genesis(GenesisOptions {
        contract_creation_policy: Some(policy),
        ..GenesisOptions::default()
    });
    start_node_with_genesis(test_genesis, finalization_blocks, Some(extension), None, 3).await
}

pub async fn setup_test_rollup_with_selective_paymaster(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<EvmBlueprint> {
    start_node_with_genesis(
        paymaster_selective_genesis(),
        finalization_blocks,
        Some(extension),
        None,
        3,
    )
    .await
}

pub async fn setup_with_simple_storage(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (
    TestRollup<EvmBlueprint>,
    sov_eth_client::SimpleStorageClient,
    u64,
) {
    setup_with_simple_storage_with_ideal_lag(finalization_blocks, extension, 3).await
}

pub async fn setup_with_simple_storage_with_ideal_lag(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    ideal_lag: u64,
) -> (
    TestRollup<EvmBlueprint>,
    sov_eth_client::SimpleStorageClient,
    u64,
) {
    let test_rollup =
        setup_test_rollup_with_ideal_lag(finalization_blocks, extension, ideal_lag).await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;
    let simple_storage = crate::common::clients::create_simple_storage_client(
        test_rollup.http_addr,
        crate::common::constants::SENDER_PRIV_KEY,
    )
    .await;
    (test_rollup, simple_storage, config_value!("CHAIN_ID"))
}
