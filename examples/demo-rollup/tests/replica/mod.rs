mod db_elected;
mod proofs;
mod recovery;
mod replica_gets_txs_from_master;
mod replica_node_lag;
mod replica_partitioned_db;
mod replica_registers_in_db;
mod root_hash_checker;
mod start_stop;
mod toxi_proxy_helper;

pub use crate::external_mock_da::{start_external_mock_da, ExternalDa};
use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_paths;
use crate::test_helpers::test_genesis_source;
use demo_stf::genesis_config::create_genesis_config;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_api_spec::types;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::SequencerKindConfig;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::OperatingMode;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_modules_stf_blueprint::GenesisParams;
use sov_proxy_utils::ClusterInfo;
use sov_proxy_utils::ClusterInfoService;
use sov_proxy_utils::RootHashCheck;
use sov_proxy_utils::RootHashConsistency;
use sov_sequencer::preferred::ConfiguredNodeRole;
use sov_sequencer::preferred::RecoveryStrategy;
use sov_sequencer::SequencerRole;
use sov_shutdown::SecondaryShutdownController;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::postgres::CreatePostgresError;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::PostgresData;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::Duration;

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
const AMOUNT: u128 = 100;

fn random_address() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

fn periodic_block_producing() -> BlockProducingConfig {
    BlockProducingConfig::Periodic {
        block_time_ms: TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS * 2,
    }
}

async fn start_rollup_with_connection_string(
    addr: SocketAddr,
    postgres: Option<(Arc<PostgresData>, String, ConfiguredNodeRole)>,
    postgres_connection_override: Option<String>,
) -> TestRollup<ExternalMockDemoRollup<Native>> {
    let genesis = test_genesis_source(OperatingMode::Operator);
    RollupBuilder::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{addr}"),
        },
        postgres,
    )
    .await
    .set_config(move |c| {
        c.blob_processing_timeout_secs = 300;
        match &mut c.sequencer_config {
            SequencerKindConfig::Standard(_) => {
                panic!("Expected preferred sequencer config");
            }
            SequencerKindConfig::Preferred(p) => {
                p.num_cache_warmup_workers = 0;
                p.recovery_strategy = RecoveryStrategy::TryToSave;
                p.ideal_lag_behind_finalized_slot = 3;
                if let Some(connection_string) = postgres_connection_override.as_ref() {
                    p.postgres_config
                        .as_mut()
                        .expect(
                            "Expected postgres config when overriding postgres connection string",
                        )
                        .postgres_connection_string = connection_string.clone();
                }
            }
        }
    })
    .start_test_rollup()
    .await
    .unwrap()
}

async fn start_rollup(
    addr: SocketAddr,
    postgres: Option<(Arc<PostgresData>, String, ConfiguredNodeRole)>,
) -> TestRollup<ExternalMockDemoRollup<Native>> {
    start_rollup_with_connection_string(addr, postgres, None).await
}

/// Like [`start_rollup`], but boots the node in Zk mode with the prover
/// pipeline enabled so that aggregated proofs get produced and posted to DA.
async fn start_rollup_with_prover(
    addr: SocketAddr,
    da_service: &StorableMockDaService,
    postgres: Option<(Arc<PostgresData>, String, ConfiguredNodeRole)>,
) -> TestRollup<ExternalMockDemoRollup<Native>> {
    da_service.wait_for_height(3).await.unwrap();

    let operating_mode = OperatingMode::Zk;
    let mut runtime_config =
        create_genesis_config::<S>(&test_genesis_paths(operating_mode)).unwrap();
    runtime_config.chain_state.genesis_da_height = 3;
    let genesis = GenesisSource::CustomParams(GenesisParams {
        runtime: runtime_config,
    });

    RollupBuilder::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{addr}"),
        },
        postgres,
    )
    .await
    .enable_prover()
    .set_start_fresh_outer_proof_on_resync(true)
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 16777216;
        c.rollup_prover_config = RollupProverConfig::Prove;
        c.blob_processing_timeout_secs = 180;
        c.aggregated_proof_block_jump = 2;
        c.max_concurrent_proof_blobs = 1024;
        if let SequencerKindConfig::Preferred(seq) = &mut c.sequencer_config {
            seq.batch_execution_time_limit_millis = TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
            seq.recovery_strategy = RecoveryStrategy::TryToSave;
            seq.disable_state_root_consistency_checks = true;
            seq.num_cache_warmup_workers = 0;
            seq.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .start_test_rollup()
    .await
    .unwrap()
}

async fn send_transfers(
    start_nonce: u64,
    count: u64,
    key_and_address: PrivateKeyAndAddress<S>,
    receiver: <S as Spec>::Address,
    client: sov_api_spec::client::Client,
) {
    for n in 0..count {
        let tx = build_transfer_token_tx::<S>(
            &key_and_address.private_key,
            config_gas_token_id(),
            receiver,
            AMOUNT,
            start_nonce + n,
        );
        let mut retry_duration = Duration::from_millis(100);
        // Send the tx with retries, up to 7 attempts (about 30 seconds)
        for attempt in 1..=7 {
            match client.send_tx_to_sequencer(&tx).await {
                Ok(_) => break,
                Err(e) => {
                    if e.to_string().contains("The node fell out of sync") {
                        if attempt == 7 {
                            panic!("Failed to send tx after 7 attempts (about 30 seconds). This probably means that the test is too heavy.: {e}");
                        }
                        tokio::time::sleep(retry_duration).await;
                        retry_duration *= 2;
                    } else {
                        panic!("Unexpected error while sending tx: {e}");
                    }
                }
            }
        }
        // Wait between each successful send, to avoid overwhelming the sequencer
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_all_events_with_timeout(
    timeout: Duration,
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) {
    tokio::time::timeout(timeout, wait_for_all_events(nb_of_events, subscription))
        .await
        .unwrap();
}

async fn wait_for_all_events(
    nb_of_events: u64,
    subscription: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
) {
    for _ in 0..nb_of_events {
        let _ = subscription.next().await.unwrap();
    }
}

type Rollup = ExternalMockDemoRollup<Native>;

/// Test setup for DbElected tests with two nodes (leader and replica).
struct NodeDiscoveryTestSetup {
    postgres: Arc<PostgresData>,
    da_service: StorableMockDaService,
    da_addr: SocketAddr,
    da_shutdown: SecondaryShutdownController,
    cluster_info_service: ClusterInfoService,
}

const MAX_AGE: Duration = Duration::from_secs(10);

impl NodeDiscoveryTestSetup {
    /// Creates a new test setup with default max_age.
    /// Returns None if Docker is not supported.
    async fn new() -> Option<Self> {
        Self::new_with_max_age(MAX_AGE).await
    }

    /// Creates a new test setup with custom max_age for cluster info updates.
    /// Returns None if Docker is not supported.
    async fn new_with_max_age(max_age: Duration) -> Option<Self> {
        let postgres = match PostgresData::create_postgres().await {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let ExternalDa {
            service: da_service,
            shutdown: da_shutdown,
            addr: da_addr,
        } = start_external_mock_da(periodic_block_producing())
            .await
            .unwrap();

        let cluster_info_service =
            ClusterInfoService::spawn(postgres.connection_string(), max_age, None)
                .await
                .expect("Failed to create ClusterInfoService");

        Some(Self {
            postgres,
            da_service,
            da_shutdown,
            da_addr,
            cluster_info_service,
        })
    }

    /// Start the node.
    async fn start_node(&self, node_id: &str, role: ConfiguredNodeRole) -> TestRollup<Rollup> {
        let node = Some((self.postgres.clone(), node_id.into(), role));
        start_rollup(self.da_addr, node).await
    }

    /// Start the node with the Zk prover pipeline enabled.
    async fn start_node_with_prover(
        &self,
        node_id: &str,
        role: ConfiguredNodeRole,
    ) -> TestRollup<Rollup> {
        let node = Some((self.postgres.clone(), node_id.into(), role));
        start_rollup_with_prover(self.da_addr, &self.da_service, node).await
    }

    async fn shutdown(self) {
        self.cluster_info_service.shutdown();
        self.da_shutdown.shutdown();
    }

    async fn wait_for_cluster_change(&mut self) -> ClusterInfo {
        self.wait_for_cluster_change_with_timeout(Duration::from_secs(2))
            .await
    }

    async fn wait_for_cluster_change_with_timeout(&mut self, timeout: Duration) -> ClusterInfo {
        self.cluster_info_service
            .wait_for_update_with_timeout(timeout)
            .await
            .expect("Failed to receive cluster info update")
    }

    async fn wait_for_root_hash_check_with_timeout(&mut self, timeout: Duration) -> RootHashCheck {
        tokio::time::timeout(timeout, async {
            let receiver = &mut self.cluster_info_service.node_checker_task.receiver;
            receiver
                .changed()
                .await
                .expect("Root hash checker channel closed");

            receiver.borrow_and_update().clone().root_hash_check
        })
        .await
        .expect("Timed out waiting for root hash checker update")
    }
}

async fn establish_leader_and_replica(
    node_1: TestRollup<Rollup>,
    node_2: TestRollup<Rollup>,
) -> (TestRollup<Rollup>, TestRollup<Rollup>) {
    // Discover roles via the /sequencer/role endpoint
    let role1 = node_1.sequencer_role().await.unwrap();
    let role2 = node_2.sequencer_role().await.unwrap();

    // Determine which node is the leader and which is the replica
    let (leader, replica) = match (role1, role2) {
        (SequencerRole::BatchProducer, SequencerRole::PgSyncReplica) => (node_1, node_2),
        (SequencerRole::PgSyncReplica, SequencerRole::BatchProducer) => (node_2, node_1),
        _ => {
            panic!("Expected one BatchProducer and one PgSyncReplica, got {role1:?} and {role2:?}")
        }
    };

    (leader, replica)
}

async fn verify_replica_processes_tx(
    leader: &TestRollup<Rollup>,
    replica: &TestRollup<Rollup>,
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    token_id: sov_bank::TokenId,
    receiver_addr: <S as Spec>::Address,
    nonce: u64,
) {
    let tx = build_transfer_token_tx::<S>(key, token_id, receiver_addr, AMOUNT, nonce);

    let mut event_subscription = replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    leader.send_tx_to_sequencer(&tx).await.unwrap();
    wait_for_all_events_with_timeout(Duration::from_millis(3500), 1, &mut event_subscription).await;
}
