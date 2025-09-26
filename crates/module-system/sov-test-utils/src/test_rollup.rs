use crate::postgres::connection_string_from_postgres_container;
use std::net::SocketAddr;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::postgres::create_postgres_container;
use crate::postgres::PostgresImage;
use anyhow::Context;
use derivative::Derivative;
use serde::Deserialize;
use sov_api_spec::WsSubscription;
use sov_blob_sender::BlobExecutionStatus;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_cli::NodeClient;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::storable::service::StorableMockDaService;
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
use testcontainers::ContainerAsync;
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio::time::Duration;

use crate::{
    TEST_DEFAULT_PROVER_ADDRESS, TEST_DEFAULT_SEQUENCER_ADDRESS, TEST_MAX_BATCH_SIZE,
    TEST_MAX_CONCURRENT_BLOBS, TEST_NUM_CACHE_WARMUP_WORKERS,
};

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

/// Various configuration options for [`RollupBuilder`].
#[allow(missing_docs)]
#[derive(Clone)]
pub struct RollupBuilderConfig<S: Spec, StoragePath = Arc<tempfile::TempDir>> {
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
    /// This is wrapped in an [`Arc`] to enable re-use of the same directory
    /// when dropping a [`TestRollup`] and creating a new one. The pattern
    /// looks something like this:
    ///
    ///  1. Instantiate a [`RollupBuilder`].
    ///  2. Clone its [`RollupBuilderConfig::storage`] and store it for later use.
    ///  3. Run some tests against the [`TestRollup`], then call
    ///     [`TestRollup::shutdown`].
    ///  4. Instantiate a new [`RollupBuilder`] and set its
    ///     [`RollupBuilderConfig::storage`] to the original directory.
    ///  6. Voila, your data is still there and you can test node behavior after
    ///     a restart.
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
}

/// A one-stop shop for building entire rollups and starting them in the
/// background to test node APIs.
#[derive(Derivative)]
#[derivative(Clone(bound = "StoragePath: Clone"))]
pub struct RollupBuilder<R: FullNodeBlueprint<Native>, StoragePath = Arc<tempfile::TempDir>> {
    genesis: GenesisSource<R::Spec, R::Runtime>,
    da_config: MockDaConfig,
    config: RollupBuilderConfig<R::Spec, StoragePath>,
    postgres_container_opt: Option<Arc<ContainerAsync<PostgresImage>>>,
    with_secondary_sequencer: Option<MockAddress>,
}

impl<R: FullNodeBlueprint<Native>> RollupBuilder<R> {
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
            Arc::new(tempfile::tempdir().unwrap()),
            true,
        )
    }

    /// Uses the preferred sequencer with Postgres as a database.
    pub async fn with_postgres_sequencer(mut self) -> anyhow::Result<Self> {
        let postgres =
            create_postgres_container(&self.config.storage.as_path().join("postgres_data")).await
            .with_context(|| "Failed to start Postgres container. This is most likely because (1) the Docker daemon is not running or (2) Docker Desktop doesn't have file sharing permissions to the repository directory")?;

        let postgres_connection_string =
            connection_string_from_postgres_container(&postgres).await?;

        match &mut self.config.sequencer_config {
            SequencerKindConfig::Preferred(ref mut config) => {
                config.postgres_connection_string = Some(postgres_connection_string);
                self.postgres_container_opt = Some(Arc::new(postgres));
            }
            _ => panic!("Can't use Postgres with a non-preferred sequencer"),
        }

        Ok(self)
    }
}

/// A type that can be used as a [`Path`].
// We need a custom trait because Arc<T> doesn't implement AsRef<Path>
// even if T does.
pub trait AsPath: Clone {
    /// Returns a [`Path`] reference.
    fn as_path(&self) -> &Path;
}

// We also can't blanket impl AsPath because rustc complains that TempDir might add an `Arc<Tempdir>: AsRef<Path>` implementation in the future.
impl AsPath for Arc<tempfile::TempDir> {
    fn as_path(&self) -> &Path {
        self.as_ref().as_ref()
    }
}

impl AsPath for PathBuf {
    fn as_path(&self) -> &Path {
        self.as_path()
    }
}

impl<R: FullNodeBlueprint<Native>, StoragePath: AsPath> RollupBuilder<R, StoragePath> {
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
                MockDaConfig::sqlite_in_dir(storage_path.as_path()).unwrap()
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
            config: RollupBuilderConfig {
                max_allowed_node_distance_behind: 10,
                max_batch_size_bytes: TEST_MAX_BATCH_SIZE,
                max_concurrent_blobs: TEST_MAX_CONCURRENT_BLOBS,
                max_channel_size: 60,
                max_infos_in_db: 250 + finalization_blocks as u64,
                automatic_batch_production: true,
                sequencer_config: SequencerKindConfig::Preferred(Default::default()),
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
                }),
                num_cache_warmup_workers: TEST_NUM_CACHE_WARMUP_WORKERS,
            },
            with_secondary_sequencer: None,
        }
    }

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
    pub fn set_config(
        mut self,
        config_f: impl FnOnce(&mut RollupBuilderConfig<R::Spec, StoragePath>),
    ) -> Self {
        config_f(&mut self.config);
        self
    }

    /// Allows to modify DA configuration options.
    pub fn set_da_config(mut self, config_f: impl FnOnce(&mut MockDaConfig)) -> Self {
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

    /// If rollup needs to be restarted, this needs to be activated.
    pub fn set_persistent_da(mut self) -> Self {
        // We store DA data in the same directory as the rollup data. This
        // ensures that, when reusing the same path, we restore not only node
        // data but also DA history.
        self.da_config.connection_string =
            MockDaConfig::sqlite_in_dir(self.config.storage.as_path())
                .expect("storage folder should exist by this time");
        self
    }

    /// A reference to the storage directory the rollup will run in
    pub fn storage_path(&self) -> StoragePath {
        self.config.storage.clone()
    }
}

impl<R, StoragePath> RollupBuilder<R, StoragePath>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
    StoragePath: AsPath,
{
    /// Creates a new [`TestRollup`] and starts running it in a background Tokio
    /// task. See [`TestRollup`] for usage information.
    pub async fn start(self) -> anyhow::Result<TestRollup<R, StoragePath>> {
        let blueprint: R = Default::default();
        if let SequencerKindConfig::Preferred(sequencer_conf) = &self.config.sequencer_config {
            if self.config.rollup_prover_config.is_some()
                && !sequencer_conf.disable_state_root_consistency_checks
            {
                tracing::warn!("Prover process is enabled, but state root consistency checks are not disabled. This will cause crashes in the sequencer since proofs are created but not yet handled by the sequencer. Consider disabling one of the two options.");
            }
        }
        std::fs::create_dir_all(self.config.storage.as_path()).with_context(|| {
            format!(
                "Failed to create storage directory: {}",
                self.config.storage.as_path().display()
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

        let (secondary_test_sequencer_client, secondary_sequencer_state_sender) =
            match self.with_secondary_sequencer {
                Some(addr) => {
                    // We "keep" it because it is going to be deleted when the parent is deleted.
                    let second_sequencer_dir = tempfile::Builder::new()
                        .disable_cleanup(true)
                        .tempdir_in(self.config.storage.as_path())?;
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
            secondary_test_sequencer_client,
            _secondary_sequencer_state_sender: secondary_sequencer_state_sender,
            other_handles,
        })
    }

    fn rollup_config(&self) -> RollupConfig<<R::Spec as Spec>::Address, R::DaService> {
        RollupConfig {
            storage: RollupDbConfig::default_in_path(self.config.storage.as_path().to_path_buf()),
            runner: RunnerConfig {
                genesis_height: 0,
                da_polling_interval_ms: 30,
                da_total_timeout_secs: 3_600,
                http_config: HttpServerConfig::on_host_port(
                    &self.config.axum_host,
                    self.config.axum_port,
                ),
                concurrent_sync_tasks: Some(1),
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
                extension: self.config.extension.clone(),
            },

            monitoring: MonitoringConfig {
                telegraf_address: self.config.telegraf_address,
                max_datagram_size: None,
                max_pending_metrics: None,
            },
        }
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
}

impl<R> RollupBuilder<R, Arc<tempfile::TempDir>>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
{
    /// Creates multiple [`TestRollup`] instances with shared database infrastructure.
    /// The first rollup acts as master, subsequent ones as replicas.
    /// All instances share the same MockDA sqlite and postgres database.
    ///
    /// # Requirements
    /// - Must be called after `.with_postgres_sequencer()` to set up shared postgres; skipped in
    ///   contexts that skip postgres tests (i.e. the dev server)
    /// - Only works with a TempDir currently
    ///
    /// # Example
    /// ```ignore
    /// let rollups = RollupBuilder::new(genesis, config)
    ///     .with_postgres_sequencer().await?
    ///     .start_with_replicas(3).await?; // 1 master + 2 replicas
    /// ```
    pub async fn start_with_replicas(
        self,
        num_replicas: u64,
    ) -> anyhow::Result<Vec<TestRollup<R, Arc<tempfile::TempDir>>>> {
        if num_replicas == 0 {
            anyhow::bail!("num_replicas must be at least 1 (master + replicas)");
        }

        // Validate configuration requirements
        let SequencerKindConfig::Preferred(ref preferred_config) = self.config.sequencer_config
        else {
            panic!("Replicas can only be used with Preferred sequencer configuration. Use RollupBuilder::with_preferred_sequencer() first.");
        };

        if preferred_config.postgres_connection_string.is_none() {
            panic!("Replicas require shared postgres database. Call .with_postgres_sequencer().await? before .start_with_replicas()");
        }

        // Create base temp directory for shared infrastructure
        let base_path = self.config.storage.as_path();
        std::fs::create_dir_all(base_path).with_context(|| {
            format!(
                "Failed to create storage directory: {}",
                base_path.display()
            )
        })?;

        // Create shared MockDA sqlite file in base directory
        let shared_da_connection = format!(
            "sqlite://{}?mode=rwc",
            base_path.join("shared_mock_da.sqlite").to_string_lossy()
        );
        let da_layer = Arc::new(RwLock::new(
            StorableMockDaLayer::new_from_connection(
                &shared_da_connection,
                self.da_config.finalization_blocks,
            )
            .await?,
        ));

        let mut rollups = Vec::new();

        for i in 0..num_replicas {
            // Create instance-specific storage directory
            let instance_dir = base_path.join(format!("instance_{i}"));
            std::fs::create_dir_all(&instance_dir)?;

            // Clone builder configuration for this instance
            let mut instance_builder = RollupBuilder {
                genesis: self.genesis.clone(),
                da_config: MockDaConfig {
                    connection_string: shared_da_connection.clone(),
                    da_layer: Some(da_layer.clone()),
                    ..self.da_config.clone()
                },
                config: RollupBuilderConfig {
                    storage: Arc::new(tempfile::Builder::new().tempdir_in(&instance_dir)?),
                    ..self.config.clone()
                },
                postgres_container_opt: self.postgres_container_opt.clone(),
                with_secondary_sequencer: None, // No secondary sequencer support in replica mode
            };

            // Set replica mode for non-master instances
            if i > 0 {
                if let SequencerKindConfig::Preferred(ref mut preferred_config) =
                    instance_builder.config.sequencer_config
                {
                    preferred_config.is_replica = true;
                }
            }

            // Start this instance
            let rollup = instance_builder.start().await?;
            rollups.push(rollup);
        }

        Ok(rollups)
    }
}

/// Represents a **running** rollup node while providing access to its
/// [`DaService`] and wallet client
/// to help run end-to-end tests against its APIs.
pub struct TestRollup<R: FullNodeBlueprint<Native>, StoragePath = Arc<tempfile::TempDir>> {
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
    pub da_service: Arc<StorableMockDaService>,
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
    pub builder: RollupBuilder<R, StoragePath>,
    // Keep it open, so the secondary sequencer runs without errors
    #[allow(dead_code)]
    _secondary_sequencer_state_sender:
        Option<watch::Sender<StateUpdateInfo<<R::Spec as Spec>::Storage>>>,
}

impl<R, StoragePath> TestRollup<R, StoragePath>
where
    R: FullNodeBlueprint<Native, DaService = StorableMockDaService> + Default + 'static,
    R::Spec: Spec<Da = MockDaSpec>,
    StoragePath: AsPath,
{
    /// Default timeout for polling operations in seconds.
    pub const POLLING_TIMEOUT: u64 = 20;

    /// Pauses batch production for the preferred sequencer.
    ///
    /// Transactions accepted by the preferred sequencer after this call (and
    /// before [`TestRollup::resume_preferred_batches`]) will all be part of the
    /// same batch.
    pub async fn pause_preferred_batches(&self) {
        std::env::set_var("SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE", "1");
    }

    /// Helper to get api_client
    pub fn api_client(&self) -> &sov_api_spec::client::Client {
        &self.client.client
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

    /// Waits for the rollup to shutdown.
    pub async fn wait_for_rollup_to_shutdown(self, t: tokio::time::Duration) {
        timeout(t, self.rollup_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    /// Shuts down the rollup and waits for all background tasks to finish.
    pub async fn shutdown(self) -> anyhow::Result<RollupBuilder<R, StoragePath>> {
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
        self.client.client.is_ready().await.is_ok()
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

        timeout(Duration::from_secs(Self::POLLING_TIMEOUT), wait_loop)
            .await
            .with_context(|| {
                format!(
                    "Timeout waiting for {condition_string} after {} seconds",
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

    /// Wait until sequencer reaches a specific height.
    pub async fn wait_for_height(&self, height: u64) {
        let mut current_height = get_height(&self.client).await.unwrap();
        while current_height.get() < height {
            self.da_service.produce_block_now().await.unwrap();
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
