#![allow(dead_code, missing_docs)]
use crate::postgres::connection_string_from_postgres_container;
use std::net::SocketAddr;
use std::num::NonZero;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::postgres::create_postgres_container;
use crate::postgres::CreatePostgresError;
use crate::postgres::PostgresImage;
use crate::Transaction;
use crate::{
    TEST_DEFAULT_PROVER_ADDRESS, TEST_DEFAULT_SEQUENCER_ADDRESS, TEST_MAX_BATCH_SIZE,
    TEST_MAX_CONCURRENT_BLOBS, TEST_NUM_CACHE_WARMUP_WORKERS,
};
use anyhow::Context;
use derivative::Derivative;
use serde::Deserialize;
use sov_api_spec::types::TxInfoWithConfirmation;
use sov_api_spec::WsSubscription;
use sov_blob_sender::BlobExecutionStatus;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_cli::NodeClient;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::rpc::StorableMockDaClient;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaConfig, MockDaSpec};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::prelude::axum;
use sov_modules_api::prelude::axum::extract::Request;
use sov_modules_api::prelude::axum::ServiceExt;
use sov_modules_api::{Spec, Zkvm};
pub use sov_modules_rollup_blueprint::FullNodeBlueprint;
use sov_modules_stf_blueprint::{GenesisParams, Runtime};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{DaSyncState, SyncStatus};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::ZkvmHost;
use sov_rollup_interface::StateUpdateInfo;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::test_stateless::TestStatelessSequencer;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::{SequencerApis, SequencerConfig, SequencerKindConfig, StateUpdateNotification};
pub use sov_stf_runner::processes::RollupProverConfig;
use sov_stf_runner::{
    HttpServerConfig, MonitoringConfig, ProofManagerConfig, RollupConfig, RunnerConfig,
};
use tempfile::TempDir;
use testcontainers::ContainerAsync;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio::time::Duration;

/// Specifies how to source the genesis data for a rollup.
#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
pub enum GenesisSource<S: Spec, R: Runtime<S>> {
    /// Genesis data will be parsed from files found at the given paths.
    ///
    /// See [`FullNodeBlueprint::create_genesis_config`].
    Paths(R::GenesisInput),
    /// Genesis data provided explicitly using [`GenesisParams`].
    ///
    /// This is most useful when you're automatically generating genesis data
    /// rather than parsing it. See e.g.
    /// [`crate::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig::generate`].
    CustomParams(GenesisParams<R::GenesisConfig>),
}

#[derive(Clone)]
pub enum StoragePath {
    Tmp(Arc<tempfile::TempDir>),
    Buf(PathBuf),
}

impl StoragePath {
    pub fn path(&self) -> &Path {
        match self {
            Self::Tmp(tmp) => tmp.path(),
            Self::Buf(buf) => buf.as_path(),
        }
    }
}

/// Various configuration options for [`RollupBuilder`].
#[derive(Clone)]
pub struct RollupBuilderConfig<S: Spec> {
    pub automatic_batch_production: bool,
    pub max_allowed_node_distance_behind: u64,
    pub sequencer_config: SequencerKindConfig,
    pub prover_address: String,
    pub sequencer_address: String,
    pub aggregated_proof_block_jump: usize,
    pub max_infos_in_db: u64,
    pub max_channel_size: u64,
    pub telegraf_address: sov_stf_runner::TelegrafSocketConfig,
    pub rollup_prover_config: Option<RollupProverConfig<S::InnerZkvm>>,
    pub storage: StoragePath,
    pub axum_host: String,
    pub axum_port: u16,
    pub max_batch_size_bytes: usize,
    pub max_concurrent_blobs: usize,
    pub blob_processing_timeout_secs: u64,
    pub start_at_rollup_height: Option<RollupHeight>,
    pub stop_at_rollup_height: Option<RollupHeight>,
    pub extension: Option<SeqConfigExtension>,
    pub num_cache_warmup_workers: usize,
    pub separate_archival_db: bool,
}

/// A one-stop shop for building entire rollups and starting them in the
/// background to test node APIs.
#[derive(Clone)]
pub struct RollupBuilder<R: FullNodeBlueprint<Native>> {
    genesis: GenesisSource<R::Spec, R::Runtime>,
    da_config: <<R as FullNodeBlueprint<sov_modules_api::execution_mode::Native>>::DaService as DaService>::Config,
    config: RollupBuilderConfig<R::Spec>,
    postgres_container_opt: Option<Arc<PostgresData>>,
    with_secondary_sequencer: Option<MockAddress>,
}

impl<R: FullNodeBlueprint<Native> + Default + 'static> RollupBuilder<R> {
    /// See [`PreferredSequencerConfig::minimum_profit_per_tx`].
    pub fn with_preferred_seq_min_profit_per_tx(mut self, minimum_profit_per_tx: u128) -> Self {
        if let SequencerKindConfig::Preferred(ref mut config) = &mut self.config.sequencer_config {
            config.minimum_profit_per_tx = minimum_profit_per_tx;
        } else {
            self.config.sequencer_config =
                SequencerKindConfig::Preferred(PreferredSequencerConfig {
                    minimum_profit_per_tx,
                    ..Default::default()
                });
        }
        self
    }

    /// See [`PreferredSequencerConfig::recovery_strategy`].
    pub fn with_preferred_seq_recovery_strategy(
        mut self,
        recovery_strategy: sov_sequencer::preferred::RecoveryStrategy,
    ) -> Self {
        if let SequencerKindConfig::Preferred(ref mut config) = &mut self.config.sequencer_config {
            config.recovery_strategy = recovery_strategy;
        } else {
            self.config.sequencer_config =
                SequencerKindConfig::Preferred(PreferredSequencerConfig {
                    recovery_strategy,
                    ..Default::default()
                });
        }
        self
    }

    /// See [`RollupBuilderConfig::rollup_prover_config`].
    pub fn with_zkvm_host_args(
        mut self,
        zkvm_host_args: Arc<<<<R::Spec as Spec>::InnerZkvm as Zkvm>::Host as ZkvmHost>::HostArgs>,
    ) -> Self {
        self.config.rollup_prover_config = Some(get_appropriate_rollup_prover_config::<R::Spec>(
            zkvm_host_args,
        ));

        self.disable_state_root_consistency_checks()
    }

    /// Disable the state root consistency checks.
    pub fn disable_state_root_consistency_checks(mut self) -> Self {
        if let SequencerKindConfig::Preferred(ref mut config) = &mut self.config.sequencer_config {
            config.disable_state_root_consistency_checks = true;
        }
        self
    }

    /// Allows to modify configuration options.
    pub fn set_config(mut self, config_f: impl FnOnce(&mut RollupBuilderConfig<R::Spec>)) -> Self {
        config_f(&mut self.config);
        self
    }

    /// Allows to modify DA configuration options.
    pub fn set_da_config(
        mut self,
        config_f: impl FnOnce(&mut <<R as FullNodeBlueprint<sov_modules_api::execution_mode::Native>>::DaService as DaService>::Config),
    ) -> Self {
        config_f(&mut self.da_config);
        self
    }

    /// Sets the sequencer "kind" to [`SequencerKindConfig::Standard`].
    pub fn with_standard_sequencer(self) -> Self {
        self.set_config(|c| {
            c.sequencer_config = SequencerKindConfig::Standard(Default::default());
        })
    }

    /// Runs a secondary sequencer with [`TestStatelessSequencer`] on the same DA layer
    /// with the provided DA Address.
    pub fn with_secondary_sequencer(mut self, sequencer_da_address: MockAddress) -> Self {
        self.with_secondary_sequencer = Some(sequencer_da_address);
        self
    }

    /// A reference to the storage directory the rollup will run in
    pub fn storage_path(&self) -> StoragePath {
        self.config.storage.clone()
    }

    pub async fn start_test_rollup(self) -> anyhow::Result<TestRollup<R>> {
        let blueprint: R = Default::default();
        if let SequencerKindConfig::Preferred(sequencer_conf) = &self.config.sequencer_config {
            if self.config.rollup_prover_config.is_some()
                && !sequencer_conf.disable_state_root_consistency_checks
            {
                tracing::warn!("Prover process is enabled, but state root consistency checks are not disabled. This will cause crashes in the sequencer since proofs are created but not yet handled by the sequencer. Consider disabling one of the two options.");
            }
        }
        std::fs::create_dir_all(self.config.storage.path()).with_context(|| {
            format!(
                "Failed to create storage directory: {}",
                self.config.storage.path().display()
            )
        })?;

        let rollup_config = self.rollup_config();
        let rollup = match &self.genesis {
            GenesisSource::Paths(genesis_paths) => {
                blueprint
                    .create_new_rollup(
                        genesis_paths,
                        rollup_config.clone(),
                        self.config.rollup_prover_config.clone(),
                        self.config.start_at_rollup_height,
                        self.config.stop_at_rollup_height,
                    )
                    .await?
            }
            GenesisSource::CustomParams(genesis_params) => {
                blueprint
                    .create_new_rollup_with_genesis_params(
                        genesis_params.clone(),
                        rollup_config.clone(),
                        self.config.rollup_prover_config.clone(),
                        self.config.start_at_rollup_height,
                        self.config.stop_at_rollup_height,
                    )
                    .await?
            }
        };

        let (rest_addr_tx, rest_addr_rx) = tokio::sync::oneshot::channel();
        let shutdown_sender = rollup.shutdown_sender.clone();

        let mut other_handles = Vec::new();
        let da_service = rollup.runner.da_service();

        if let Some(handle) = da_service.take_background_join_handle().await {
            other_handles.push(handle);
        }

        let rollup_task = tokio::spawn(async move {
            match rollup.run_and_report_addr(Some(rest_addr_tx)).await {
                Ok(()) => {
                    tracing::info!("Completed running a rollup");
                    Ok(())
                }
                Err(error) => {
                    tracing::error!(?error, "Rollup execution returned an error");
                    Err(error)
                }
            }
        });

        let rest_addr = rest_addr_rx.await?;

        let rest_url = format!("http://{}:{}", rest_addr.ip(), rest_addr.port());
        let client = match NodeClient::new(&rest_url).await {
            Ok(client) => client,
            Err(e) => {
                tracing::warn!(
                    "Unable to instantiate standard NodeClient for node at {}: {e}",
                    rest_url,
                );
                NodeClient::new_unchecked(&rest_url)
            }
        };

        Ok(TestRollup {
            builder: self,
            rollup_task,
            http_addr: rest_addr,
            rollup_config,
            client,
            da_service,
            shutdown_sender,
            secondary_test_sequencer_client: None,
            _secondary_sequencer_state_sender: None,
            other_handles,
        })
    }

    pub fn rollup_config(&self) -> RollupConfig<<R::Spec as Spec>::Address, R::DaService> {
        let mut rollup_db_config =
            RollupDbConfig::default_in_path(self.config.storage.path().to_path_buf());
        if self.config.separate_archival_db {
            rollup_db_config.separate_archival_state = true;
        }
        RollupConfig {
            storage: rollup_db_config,
            runner: RunnerConfig {
                da_polling_interval_ms: 30,
                da_total_timeout_secs: 3_600,
                http_config: HttpServerConfig::on_host_port(
                    &self.config.axum_host,
                    self.config.axum_port,
                ),
                concurrent_sync_tasks: 1,
                pre_fetched_blocks_capacity: NonZero::new(3).unwrap(),
                save_tx_bodies: false,
            },
            da: self.da_config.clone(),
            proof_manager: ProofManagerConfig {
                aggregated_proof_block_jump: NonZero::new(self.config.aggregated_proof_block_jump)
                    .unwrap(),
                prover_address: FromStr::from_str(&self.config.prover_address)
                    .expect("Prover address is not valid"),
                max_number_of_transitions_in_db: NonZero::new(self.config.max_infos_in_db).unwrap(),
                max_number_of_transitions_in_memory: NonZero::new(self.config.max_channel_size)
                    .unwrap(),
            },
            sequencer: SequencerConfig {
                automatic_batch_production: self.config.automatic_batch_production,
                max_allowed_node_distance_behind: self.config.max_allowed_node_distance_behind,
                // Set ttl to zero to disable for testing. This prevents nondeterminism.
                dropped_tx_ttl_secs: 0,
                rollup_address: FromStr::from_str(&self.config.sequencer_address)
                    .expect("Sequencer address is not valid"),
                admin_addresses: vec![],
                sequencer_kind_config: self.config.sequencer_config.clone(),
                max_batch_size_bytes: self.config.max_batch_size_bytes,
                max_concurrent_blobs: self.config.max_concurrent_blobs,
                blob_processing_timeout_secs: self.config.blob_processing_timeout_secs,
                extension: self.config.extension,
            },

            monitoring: MonitoringConfig {
                telegraf_address: self.config.telegraf_address,
                max_datagram_size: None,
                max_pending_metrics: None,
            },
        }
    }

    fn default_config(
        finalization_blocks: u32,
        storage_path: StoragePath,
        is_replica: bool,
        postgres_connection_string: Option<String>,
    ) -> RollupBuilderConfig<R::Spec> {
        RollupBuilderConfig {
            max_allowed_node_distance_behind: 10,
            max_batch_size_bytes: TEST_MAX_BATCH_SIZE,
            max_concurrent_blobs: TEST_MAX_CONCURRENT_BLOBS,
            max_channel_size: 60,
            max_infos_in_db: 250 + finalization_blocks as u64,
            automatic_batch_production: true,
            sequencer_config: SequencerKindConfig::Preferred(PreferredSequencerConfig {
                is_replica,
                postgres_connection_string,
                ..Default::default()
            }),
            prover_address: TEST_DEFAULT_PROVER_ADDRESS.to_string(),
            sequencer_address: TEST_DEFAULT_SEQUENCER_ADDRESS.to_string(),
            aggregated_proof_block_jump: 1,
            rollup_prover_config: None,
            storage: storage_path,
            telegraf_address: MonitoringConfig::standard().telegraf_address,
            axum_host: "127.0.0.1".to_string(),
            axum_port: 0,
            blob_processing_timeout_secs: 60,
            start_at_rollup_height: None,
            stop_at_rollup_height: None,
            extension: Some(SeqConfigExtension {
                max_log_limit: 20000,
                response_size_limit: (1024 * 1024) - (1024 * 30), // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
            }),
            num_cache_warmup_workers: TEST_NUM_CACHE_WARMUP_WORKERS,
            separate_archival_db: true,
        }
    }
}

pub struct PostgresData {
    storage_path: TempDir,
    postgres: ContainerAsync<PostgresImage>,
    connection_string: String,
}

impl PostgresData {
    pub async fn create_postgres() -> Result<Arc<PostgresData>, CreatePostgresError> {
        let dir = tempfile::tempdir().unwrap();
        let pg = create_postgres_container(&dir.path().join("postgres_data")).await?;

        Ok(Arc::new(PostgresData {
            storage_path: dir,
            connection_string: connection_string_from_postgres_container(&pg).await?,
            postgres: pg,
        }))
    }
}

impl<R> RollupBuilder<R>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaClient> + Default + 'static,
{
    pub async fn new_with_external_da(
        is_replica: bool,
        genesis: GenesisSource<R::Spec, R::Runtime>,
        da_config: MockDaClientConfig,
        postgres_container_opt: Option<Arc<PostgresData>>,
    ) -> Self {
        let storage_path = StoragePath::Tmp(Arc::new(tempfile::tempdir().unwrap()));
        let post_str = postgres_container_opt
            .as_ref()
            .map(|p| p.connection_string.clone());

        Self {
            genesis,
            da_config,
            config: Self::default_config(0, storage_path, is_replica, post_str),
            postgres_container_opt,
            with_secondary_sequencer: None,
        }
    }
}

impl<R> RollupBuilder<R>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
{
    /// Creates a new [`RollupBuilder`] with automatic [`StorableMockDaService`]
    /// configuration.
    pub fn new(
        genesis: GenesisSource<R::Spec, R::Runtime>,
        block_producing: BlockProducingConfig,
        finalization_blocks: u32,
    ) -> Self {
        Self::new_with_storage_path(
            genesis,
            block_producing,
            finalization_blocks,
            StoragePath::Tmp(Arc::new(tempfile::tempdir().unwrap())),
            true,
        )
    }

    /// Creates a new [`RollupBuilder`] with automatic [`StorableMockDaService`]
    /// configuration.
    pub fn new_with_storage_path(
        genesis: GenesisSource<R::Spec, R::Runtime>,
        block_producing: BlockProducingConfig,
        finalization_blocks: u32,
        storage_path: StoragePath,
        in_memory_da: bool,
    ) -> Self {
        let da_config = MockDaConfig {
            // This will be set later based on the storage path. In case of a bug,
            // SQLite will simply fail to open the file and we'll immediately get a
            // panic, so it's not dangerous.
            connection_string: if in_memory_da {
                MockDaConfig::sqlite_in_memory()
            } else {
                MockDaConfig::sqlite_in_dir(storage_path.path()).unwrap()
            },
            // This value is important and should match `examples/test-data/genesis/integration-tests/sequencer_registry.json`
            // Otherwise batches are going to be rejected in `examples/demo-rollup` tests.
            sender_address: MockAddress::new([0; 32]),
            finalization_blocks,
            block_producing,
            da_layer: None,
            randomization: None,
        };

        Self {
            genesis,
            da_config,
            postgres_container_opt: None,
            config: Self::default_config(finalization_blocks, storage_path, false, None),
            with_secondary_sequencer: None,
        }
    }

    /// Creates a new [`TestRollup`] and starts running it in a background Tokio
    /// task. See [`TestRollup`] for usage information.
    pub async fn start(self) -> anyhow::Result<TestRollup<R>> {
        let with_secondary_sequencer = self.with_secondary_sequencer;
        let storage_config = &self.config.storage.clone();
        let mut test_rollup = self.start_test_rollup().await?;

        let da_service = test_rollup.da_service.clone();

        let rollup_config = test_rollup.rollup_config.clone();
        let shutdown_sender = test_rollup.shutdown_sender.clone();
        let (secondary_test_sequencer_client, secondary_sequencer_state_sender) =
            match with_secondary_sequencer {
                Some(addr) => {
                    // We "keep" it because it is going to be deleted when the parent is deleted.
                    let second_sequencer_dir = tempfile::Builder::new()
                        .disable_cleanup(true)
                        .tempdir_in(storage_config.path())?;
                    let mut rollup_config = rollup_config.clone();
                    rollup_config.storage.path = second_sequencer_dir.path().to_path_buf();

                    let (client, sender) = Self::start_secondary_sequencer(
                        da_service.another_on_the_same_layer(addr).await,
                        rollup_config.clone(),
                        shutdown_sender.clone(),
                    )
                    .await?;
                    (Some(client), Some(sender))
                }
                None => (None, None),
            };

        test_rollup.secondary_test_sequencer_client = secondary_test_sequencer_client;
        test_rollup._secondary_sequencer_state_sender = secondary_sequencer_state_sender;

        Ok(test_rollup)
    }

    async fn start_secondary_sequencer(
        secondary_da_service: StorableMockDaService,
        rollup_config: RollupConfig<<R::Spec as Spec>::Address, R::DaService>,
        shutdown_sender: tokio::sync::watch::Sender<()>,
    ) -> anyhow::Result<(
        sov_api_spec::client::Client,
        watch::Sender<StateUpdateInfo<<R::Spec as Spec>::Storage>>,
    )> {
        let mut shutdown_receiver = shutdown_sender.subscribe();
        let blueprint: R = Default::default();

        let mut storage_manager = blueprint.create_storage_manager(&rollup_config)?;
        let finalized_header = secondary_da_service
            .get_last_finalized_block_header()
            .await?;
        let (storage, ledger_state) = storage_manager.create_state_after(&finalized_header)?;
        let ledger_db = LedgerDb::with_reader(ledger_state)?;

        let (sync_status_sender, _) = watch::channel(SyncStatus::START);
        let da_sync_state = Arc::new(DaSyncState {
            synced_da_height: AtomicU64::new(0),
            target_da_height: AtomicU64::new(0),
            sync_status_sender,
        });

        let state_update_info = StateUpdateInfo {
            storage: storage.clone(),
            ledger_reader: ledger_db.clone_reader(),
            next_event_number: 0,
            next_tx_number: 0,
            slot_number: SlotNumber::ONE,
            latest_finalized_slot_number: SlotNumber::ONE,
            sync_status: da_sync_state.status(),
        };

        let (sender, state_update_receiver) = watch::channel(state_update_info);

        let (sequencer, _background_handles) =
            TestStatelessSequencer::<R::Runtime, R::Spec, StorableMockDaService>::create(
                secondary_da_service,
                state_update_receiver,
                da_sync_state,
                &rollup_config.storage.path,
                &rollup_config.sequencer.with_seq_config(()),
                ledger_db,
                shutdown_sender,
            )
            .await?;

        let router = SequencerApis::rest_api_server(sequencer.clone(), shutdown_receiver.clone());

        let addr = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let actual_address = listener.local_addr()?;
        let actual_port = actual_address.port();

        tokio::spawn(async move {
            axum::serve(listener, ServiceExt::<Request>::into_make_service(router))
                .with_graceful_shutdown(async move {
                    shutdown_receiver.changed().await.ok();
                })
                .await
        });

        let client = sov_api_spec::client::Client::new(&format!("http://127.0.0.1:{actual_port}"));

        Ok((client, sender))
    }

    /// If rollup needs to be restarted, this needs to be activated.
    pub fn set_persistent_da(mut self) -> Self {
        // We store DA data in the same directory as the rollup data. This
        // ensures that, when reusing the same path, we restore not only node
        // data but also DA history.
        self.da_config.connection_string = MockDaConfig::sqlite_in_dir(self.config.storage.path())
            .expect("storage folder should exist by this time");
        self
    }
}

/// Represents a **running** rollup node while providing access to its
/// [`DaService`] and wallet client
/// to help run end-to-end tests against its APIs.
pub struct TestRollup<R: FullNodeBlueprint<Native>> {
    /// A wallet client that can be used to interact with the node and submit
    /// txs to the sequencer.
    pub client: NodeClient,
    /// Address of the HTTP server.
    pub http_addr: SocketAddr,
    /// The rollup config used to run the rollup.
    pub rollup_config: RollupConfig<<R::Spec as Spec>::Address, R::DaService>,
    /// A copy of the [`DaService`]
    /// that the node uses.
    ///
    /// You can use it to query DA layer information or directly submit blobs,
    /// bypassing the sequencer.
    pub da_service:
        Arc<<R as FullNodeBlueprint<sov_modules_api::execution_mode::Native>>::DaService>,
    /// Allows programmatically initialize shutdown of the test-rollup.
    /// Used for checking graceful shutdown and restart.
    pub shutdown_sender: watch::Sender<()>,
    /// Used for cleanup/shutdown logic.
    pub rollup_task: JoinHandle<anyhow::Result<()>>,
    /// For optional handles to background tasks.
    pub other_handles: Vec<JoinHandle<()>>,
    /// In case the rollup was started with a secondary sequencer, this is the
    /// client that can be used to submit transactions.
    pub secondary_test_sequencer_client: Option<sov_api_spec::client::Client>,
    #[allow(missing_docs)]
    pub builder: RollupBuilder<R>,
    // Keep it open, so the secondary sequencer runs without errors
    #[allow(dead_code)]
    _secondary_sequencer_state_sender:
        Option<watch::Sender<StateUpdateInfo<<R::Spec as Spec>::Storage>>>,
}

impl<R> TestRollup<R>
where
    R: FullNodeBlueprint<Native> + Default + 'static,
{
    /// Default timeout for polling operations in seconds.
    pub const POLLING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

    /// Helper to get api_client
    pub fn api_client(&self) -> &sov_api_spec::client::Client {
        &self.client.client
    }

    /// Waits for the rollup to shutdown.
    pub async fn wait_for_rollup_to_shutdown(self, t: tokio::time::Duration) {
        timeout(t, self.rollup_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    /// Waits for the rollup to shutdown.
    pub async fn wait_for_rollup_to_shutdown_with_result(
        self,
        t: tokio::time::Duration,
    ) -> anyhow::Result<()> {
        timeout(t, self.rollup_task)
            .await
            .expect("Failed to join rollup task before timeout.")
            .expect("Rollup task panicked.")
    }

    /// Waits for the rollup to shutdown.
    pub async fn wait_for_rollup_to_crash(self, t: tokio::time::Duration) -> anyhow::Result<()> {
        timeout(t, self.rollup_task)
            .await
            .expect("Failed to join rollup task before timeout.")
            .expect_err("Rollup task should have crashed");
        Ok(())
    }

    /// Shuts down the rollup and waits for all background tasks to finish.
    pub async fn shutdown(self) -> anyhow::Result<RollupBuilder<R>> {
        if let Err(error) = self.shutdown_sender.send(()) {
            tracing::info!(%error, "shutdown triggered elsewhere, this is probably OK");
        }
        self.rollup_task.await.expect("Can't join rollup task")?;

        for handle in self.other_handles {
            handle.await.expect("Can't join other handles");
        }

        Ok(self.builder)
    }

    /// Returns true if any of the rollup tasks have finished.
    pub fn is_rollup_crashed(&self) -> bool {
        if self.rollup_task.is_finished() {
            return true;
        }

        self.other_handles.iter().any(|handle| handle.is_finished())
    }

    /// Force closes the current batch.
    pub async fn force_close_batch(&self) -> anyhow::Result<()> {
        self.client
            .http_post("/sequencer/test-utils/force-close-batch")
            .await?;
        Ok(())
    }

    /// Subscribe to state update completion notifications.
    pub async fn subscribe_state_updates(&self) -> WsSubscription<StateUpdateNotification> {
        self.client
            .client
            .subscribe_to_ws::<StateUpdateNotification>("/sequencer/test-utils/state-updates/ws")
            .await
    }

    /// Subscribe to blobs from the blob sender.
    pub async fn subscribe_to_blobs_from_blob_sender(
        &self,
    ) -> WsSubscription<BlobExecutionStatus<MockDaSpec>> {
        self.client
            .client
            .subscribe_to_ws::<BlobExecutionStatus<MockDaSpec>>("/sequencer/test-utils/blobs/ws")
            .await
    }

    /// Checks if the sequencer is ready without waiting.
    pub async fn is_sequencer_ready(&self) -> bool {
        match self.client.client.is_ready().await {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(?error, "Sequencer is not ready");
                false
            }
        }
    }

    /// Polls the sequencer until is_ready() returns Err(). Useful when you expect the sequencer to
    /// go into resync/recovery/startup mode, to avoid wait_for_sequencer_ready() from resolving
    /// _before_ the sequencer becomes unready.
    ///
    /// Times out after TestRollup::POLLING_TIMEOUT seconds.
    pub async fn wait_for_sequencer_not_ready(&self) -> anyhow::Result<()> {
        self.wait_for_sequencer_state(false).await
    }

    /// Polls the sequencer until is_ready() returns Ok(()).
    ///
    /// Times out after TestRollup::POLLING_TIMEOUT seconds.
    pub async fn wait_for_sequencer_ready(&self) -> anyhow::Result<()> {
        self.wait_for_sequencer_state(true).await
    }

    /// Generic helper for waiting on a condition with timeout and polling.
    ///  * condition_string: inserted into "Timeout waiting for {condition_string}", format accordingly
    async fn wait_for_condition<F, Fut>(
        &self,
        mut condition_check: F,
        condition_string: &str,
    ) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<bool>>,
    {
        let wait_loop = async {
            loop {
                match condition_check().await {
                    Ok(true) => return Ok(()),
                    Ok(false) => tokio::time::sleep(Duration::from_millis(100)).await,
                    Err(e) => return Err(e),
                }
            }
        };

        timeout(Self::POLLING_TIMEOUT, wait_loop)
            .await
            .with_context(|| {
                format!(
                    "Timeout waiting for {condition_string} after {:?}",
                    Self::POLLING_TIMEOUT
                )
            })?
    }

    /// Helper that waits for the sequencer to reach either a ready or un-ready state.
    async fn wait_for_sequencer_state(&self, wait_for_ready: bool) -> anyhow::Result<()> {
        let condition_name = if wait_for_ready {
            "sequencer to be ready"
        } else {
            "sequencer to be not-ready"
        };
        self.wait_for_condition(
            || async { Ok(self.is_sequencer_ready().await == wait_for_ready) },
            condition_name,
        )
        .await
    }

    /// Waits for the node to finish syncing with the DA layer.
    ///
    /// Times out after TestRollup::POLLING_TIMEOUT seconds.
    pub async fn wait_for_node_synced(&self) -> anyhow::Result<()> {
        self.wait_for_condition(
            || async {
                let response = self.client.client.get_sync_status().await?;
                Ok(matches!(
                    response.into_inner(),
                    sov_api_spec::types::SyncStatus::Synced { .. }
                ))
            },
            "node to sync",
        )
        .await
    }

    /// Pauses batch production for the preferred sequencer.
    ///
    /// Transactions accepted by the preferred sequencer after this call (and
    /// before [`TestRollup::resume_preferred_batches`]) will all be part of the
    /// same batch.
    pub async fn pause_preferred_batches(&self) {
        std::env::set_var("SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE", "1");
    }

    /// Resumes batch production after [`TestRollup::pause_preferred_batches`].
    ///
    /// Note: calling this method MAY NOT immediately produce a batch.
    pub async fn resume_preferred_batches(&self) {
        assert_eq!(
            std::env::var("SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE").unwrap(),
            "1",
            "Resuming but it was never paused in the first place",
        );

        std::env::remove_var("SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE");
    }

    pub async fn height(&self) -> RollupHeight {
        get_height(&self.client).await.unwrap()
    }

    /// Wait until sequencer reaches a specific height.
    pub async fn wait_for_height(&self, height: u64) {
        let mut current_height = get_height(&self.client).await.unwrap();
        while current_height.get() < height {
            tokio::time::sleep(Duration::from_millis(100)).await;
            current_height = get_height(&self.client).await.unwrap();
        }
    }

    /// Waits until the sequencer advances by the given number of blocks.
    pub async fn wait_for_next_blocks(&self, delta: u64) {
        let current_height = get_height(&self.client).await.unwrap();
        let end_height = current_height.get() + delta;
        self.wait_for_height(end_height).await;
    }

    pub async fn send_tx_to_sequencer(
        &self,
        tx: &Transaction<R::Runtime, R::Spec>,
    ) -> Result<TxInfoWithConfirmation, anyhow::Error> {
        let resp = self.client.client.send_tx_to_sequencer(&tx).await?;
        Ok(resp.into_inner())
    }
}

impl<R> TestRollup<R>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
{
    /// Restarts the rollup.
    pub async fn restart(self) -> anyhow::Result<Self> {
        self.restart_with_heights(None, None).await
    }

    /// Restarts the rollup. With an option to stop at a specific height.
    pub async fn restart_with_heights(
        self,
        start_at_height: Option<RollupHeight>,
        stop_at_height: Option<RollupHeight>,
    ) -> anyhow::Result<Self> {
        let builder = self.shutdown().await?;
        let in_memory = MockDaConfig::sqlite_in_memory();
        if builder.da_config.connection_string.contains(&in_memory) {
            anyhow::bail!("Cannot restart in-memory DA, call `set_persistent_da` on RollupBuilder before starting");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let builder = builder.set_config(|c| {
            c.start_at_rollup_height = start_at_height;
            c.stop_at_rollup_height = stop_at_height;
        });
        let rollup = builder.start().await?;
        Ok(rollup)
    }
}

/// Reads and parses a private key from the test data directory.
pub fn read_private_key<S: Spec>(suffix: &str) -> PrivateKeyAndAddress<S> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let private_keys_dir = Path::new(&manifest_dir).join("../test-data/keys");

    let data = std::fs::read_to_string(private_keys_dir.join(suffix))
        .expect("Unable to read file to string");

    let key_and_address: PrivateKeyAndAddress<S> =
        serde_json::from_str(&data).unwrap_or_else(|_| {
            panic!("Unable to convert data {} to PrivateKeyAndAddress", &data);
        });

    assert!(
        key_and_address.is_matching_to_default(),
        "Inconsistent key data"
    );

    key_and_address
}

/// Parses [`RollupProverConfig`] from its env. variable.
pub fn get_appropriate_rollup_prover_config<S: Spec>(
    host_args: Arc<<<S::InnerZkvm as Zkvm>::Host as ZkvmHost>::HostArgs>,
) -> RollupProverConfig<S::InnerZkvm> {
    let skip_guest_build = std::env::var("SKIP_GUEST_BUILD").unwrap_or_else(|_| "0".to_string());
    if skip_guest_build == "1" {
        RollupProverConfig::Skip
    } else {
        RollupProverConfig::Execute(host_args)
    }
}

/// Get rollup height
pub async fn get_height(client: &NodeClient) -> anyhow::Result<RollupHeight> {
    #[derive(Deserialize, Debug)]
    struct Data {
        value: (u64, u64),
    }

    let url = "/modules/chain-state/state/current-heights";
    let response = client.http_get(url).await?;
    let heights: Data = serde_json::from_str(&response)?;
    Ok(RollupHeight::new(heights.value.0))
}
