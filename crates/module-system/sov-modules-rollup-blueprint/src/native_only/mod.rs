mod endpoints;
pub mod logging;
pub mod proof_sender;
mod telemetry;
mod wallet;
use anyhow::Context;
use async_trait::async_trait;
pub use endpoints::*;
use futures::future;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::{DeltaReader, SchemaBatch};
use sov_modules_api::capabilities::{
    ChainState, HasCapabilities, HasKernel, ProofProcessor, RollupHeight,
};
use sov_modules_api::execution_mode::ExecutionMode;
use sov_modules_api::provable_height_tracker::MaximumProvableHeight;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{
    CodeCommitmentFor, DaSpec, NodeEndpoints, OperatingMode, ProofSender, Spec, StateCheckpoint,
    VersionReader,
};
use sov_modules_api::{GenesisParamsTrait, ModuleExecutionConfig};
use sov_modules_stf_blueprint::{GenesisParams, Runtime as RuntimeTrait, StfBlueprint};
use sov_rollup_full_node_interface::DaSyncState;
use sov_rollup_full_node_interface::StateChannel;
use sov_rollup_full_node_interface::StateUpdateInfo;
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::ProvableHeightTracker;
use sov_sequencer::preferred::PreferredSequencer;
use sov_sequencer::standard::StdSequencer;
use sov_sequencer::{ProofBlobSender, Sequencer, SequencerApis, SequencerKindConfig};
use sov_shutdown::{PrimaryShutdownController, SecondaryShutdownController};
use sov_state::storage::NativeStorage;
use sov_state::Storage;
use sov_stf_runner::processes::{
    start_op_workflow_in_background, start_operator_workflow_in_background,
    start_zk_workflow_in_background, ProverService, RollupProverConfig,
};
use sov_stf_runner::{
    initialize_state, query_state_update_info, CorsConfiguration, RollupConfig,
    StateTransitionRunner,
};
use sov_stf_runner::{make_da_sync_state, DaServiceWithCachedFinalizedHeaders};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal::unix::SignalKind;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;
pub use wallet::*;

/// Commit hash of this rollup
pub const GIT_COMMIT_HASH: &str = env!("GIT_COMMIT_HASH");
use crate::RollupBlueprint;

/// Specifies how to source the genesis data for a rollup.
///
/// The source is only consulted when the rollup state is empty. On a restart
/// with populated state, genesis data is intentionally never read: the values
/// needed at startup are recovered from chain state instead, so genesis files
/// are not required to match the current binary's `GenesisConfig` after a
/// hard fork.
pub enum GenesisSource<S: Spec, R: RuntimeTrait<S>> {
    /// Genesis data will be parsed from files found at the given paths.
    ///
    /// See [`FullNodeBlueprint::create_genesis_config`].
    Paths(R::GenesisInput),
    /// Genesis data provided explicitly using [`GenesisParams`].
    ///
    /// This is most useful when you're automatically generating genesis data
    /// rather than parsing it.
    CustomParams(GenesisParams<R::GenesisConfig>),
}

impl<S: Spec, R: RuntimeTrait<S>> Clone for GenesisSource<S, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Paths(paths) => Self::Paths(paths.clone()),
            Self::CustomParams(params) => Self::CustomParams(params.clone()),
        }
    }
}

/// This trait defines how to create all the necessary dependencies required by a rollup.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
#[async_trait]
pub trait FullNodeBlueprint<M: ExecutionMode>: RollupBlueprint<M> {
    /// Data Availability service.
    type DaService: DaService<Spec = <Self::Spec as Spec>::Da, Error = anyhow::Error>;

    /// Manager for the native storage lifecycle.
    type StorageManager: HierarchicalStorageManager<
        <Self::Spec as Spec>::Da,
        StfState = <Self::Spec as Spec>::Storage,
        StfChangeSet = <<Self::Spec as Spec>::Storage as Storage>::ChangeSet,
        LedgerState = DeltaReader,
        LedgerChangeSet = SchemaBatch,
    >;

    /// Prover service.
    type ProverService: ProverService<
        StateRoot = <<Self::Spec as Spec>::Storage as Storage>::Root,
        Witness = <<Self::Spec as Spec>::Storage as Storage>::Witness,
        DaService = Self::DaService,
    >;

    /// Serialize proof blob and adds metadata needed for verification.
    type ProofSender: ProofSender + 'static;

    /// Creates RPC methods and REST APIs for the rollup.
    async fn create_endpoints(
        &self,
        state_update_receiver: StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        sync_status_receiver: tokio::sync::watch::Receiver<SyncStatus>,
        primary_shutdown: PrimaryShutdownController,
        ledger_db: &LedgerDb,
        sequencer: &SequencerCreationReceipt<Self::Spec>,
        da_service: &Self::DaService,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints>;

    /// Creates GenesisConfig from genesis files.
    #[allow(clippy::type_complexity)]
    fn create_genesis_config(
        &self,
        rt_genesis_paths: &<Self::Runtime as RuntimeTrait<Self::Spec>>::GenesisInput,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<GenesisParams<<Self::Runtime as RuntimeTrait<Self::Spec>>::GenesisConfig>>
    {
        let rt_genesis =
            <Self::Runtime as RuntimeTrait<Self::Spec>>::genesis_config(rt_genesis_paths)
                .with_context(|| {
                    format!("Failed to read rollup genesis from {rt_genesis_paths:?}")
                })?;

        Ok(GenesisParams {
            runtime: rt_genesis,
        })
    }

    /// Creates an instance of [`DaService`].
    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        secondary_shutdown_controller: &SecondaryShutdownController,
    ) -> Self::DaService;

    /// Creates an instance of [`ProverService`].
    ///
    /// Returns the prover service together with the `final_slot_number` of the
    /// latest aggregated proof persisted in the ledger DB (if any). The caller
    /// uses this slot to anchor the STF-info stream so the next aggregation is
    /// contiguous with the proof on disk.
    async fn create_prover_service(
        &self,
        prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        da_service: &Self::DaService,
        ledger_db: &LedgerDb,
        start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<(Self::ProverService, Option<SlotNumber>)>;

    /// Creates an instance of [`Self::StorageManager`].
    /// Panics if initialization fails.
    ///
    /// `witness_generation` indicates whether the storage manager should generate witnesses
    /// for ZK proving. This is a node-level decision known at startup (true when a prover
    /// config exists).
    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        witness_generation: bool,
    ) -> anyhow::Result<Self::StorageManager>;

    /// Instantiates [`FullNodeBlueprint::ProofSender`].
    fn create_proof_sender(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        sequencer: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender>;

    /// Computes the inner (state-transition) and outer (aggregation) code commitments
    /// for this rollup's zkVM(s), typically derived from the guest ELF(s).
    fn compute_code_commitments() -> anyhow::Result<(
        CodeCommitmentFor<<Self::Spec as Spec>::InnerZkvm>,
        CodeCommitmentFor<<Self::Spec as Spec>::OuterZkvm>,
    )> {
        anyhow::bail!("compute_code_commitments not supported.")
    }

    /// Creates an instance of a LedgerDb.
    fn create_ledger_db(
        &self,
        ledger_state: <Self::StorageManager as HierarchicalStorageManager<
            <Self::Spec as Spec>::Da,
        >>::LedgerState,
    ) -> anyhow::Result<LedgerDb> {
        LedgerDb::with_reader(ledger_state)
    }

    /// Creates a new rollup.
    async fn create_new_rollup(
        &self,
        runtime_genesis_paths: &<Self::Runtime as RuntimeTrait<Self::Spec>>::GenesisInput,
        rollup_config: RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        prover_config: RollupProverConfig,
        start_at_rollup_height: Option<RollupHeight>,
        stop_at_rollup_height: Option<RollupHeight>,
        exec_config: Option<<<Self::Runtime as RuntimeTrait<Self::Spec>>::ModuleExecutionConfig as ModuleExecutionConfig>::Input>,
        start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<Rollup<Self, M>>
    where
        <Self::Spec as Spec>::Storage: NativeStorage,
    {
        self.create_new_rollup_with_genesis_source(
            GenesisSource::Paths(runtime_genesis_paths.clone()),
            rollup_config,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
            exec_config,
            start_fresh_outer_proof_on_resync,
        )
        .await
    }

    /// Injects additional HTTP APIs for the sequencer.
    async fn sequencer_additional_apis<Seq>(
        &self,
        _sequencer: Seq,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _primary_shutdown: PrimaryShutdownController,
        _sequencer_da_address: <<Self::Spec as Spec>::Da as DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        Ok(NodeEndpoints::default())
    }

    /// Creates a new sequencer and provides a [`SequencerCreationReceipt`] with
    /// some information about said sequencer.
    ///
    #[allow(clippy::too_many_arguments)]
    async fn create_sequencer(
        &self,
        state_update_receiver: watch::Receiver<StateUpdateInfo<<Self::Spec as Spec>::Storage>>,
        da_sync_state: Arc<DaSyncState>,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        ledger_db: &LedgerDb,
        api_ledger_db: &LedgerDb,
        da_service: &Self::DaService,
        primary_shutdown: PrimaryShutdownController,
        stop_at_rollup_height: Option<RollupHeight>,
        bind_addr: SocketAddr,
    ) -> anyhow::Result<SequencerCreationReceipt<Self::Spec>> {
        let max_concurrent_proof_blobs = rollup_config
            .proof_manager
            .as_ref()
            .map(|p| p.max_concurrent_proof_blobs)
            .unwrap_or(0);

        match &rollup_config.sequencer.sequencer_kind_config {
            SequencerKindConfig::Standard(seq_config) => {
                let (sequencer, background_handles) =
                    StdSequencer::<Self::Spec, Self::Runtime, Self::DaService>::create(
                        da_service.clone(),
                        state_update_receiver.clone(),
                        da_sync_state,
                        &rollup_config.storage.path,
                        &rollup_config.sequencer.with_seq_config(seq_config.clone()),
                        max_concurrent_proof_blobs,
                        ledger_db.clone(),
                        api_ledger_db.clone(),
                        primary_shutdown.clone(),
                    )
                    .await?;

                let da_address = da_service.get_signer().await.context(
                    "Full node with standard sequencer require DaService with signer support",
                )?;
                let mut endpoints = self
                    .sequencer_additional_apis(
                        sequencer.clone(),
                        rollup_config,
                        primary_shutdown.clone(),
                        da_address,
                    )
                    .await?;
                endpoints.axum_router = endpoints.axum_router.merge(
                    SequencerApis::rest_api_server(sequencer.clone(), primary_shutdown),
                );

                Ok(SequencerCreationReceipt {
                    api_state: sequencer.api_state(),
                    endpoints,
                    background_handles,
                    proof_sender: Arc::new(sequencer),
                    api_ledger_db: api_ledger_db.clone(),
                    da_address,
                    is_replica: false,
                })
            }
            SequencerKindConfig::Preferred(seq_config) => {
                let (sequencer, background_handles) =
                    PreferredSequencer::<Self::Spec, Self::Runtime, Self::DaService>::create(
                        da_service.clone(),
                        state_update_receiver.clone(),
                        &rollup_config.storage.path,
                        rollup_config
                            .sequencer
                            .with_seq_config(seq_config.clone())
                            .clone(),
                        max_concurrent_proof_blobs,
                        ledger_db.clone(),
                        api_ledger_db.clone(),
                        primary_shutdown.clone(),
                        stop_at_rollup_height,
                        bind_addr,
                    )
                    .await?;
                let seq_role = sequencer.sequencer_role().await?;

                let da_address = da_service.get_signer().await.context(
                    "Full node with preferred sequencer require DaService with signer support",
                )?;
                let mut endpoints = self
                    .sequencer_additional_apis(
                        sequencer.clone(),
                        rollup_config,
                        primary_shutdown.clone(),
                        da_address,
                    )
                    .await?;
                endpoints.axum_router = endpoints.axum_router.merge(
                    SequencerApis::rest_api_server(sequencer.clone(), primary_shutdown),
                );

                Ok(SequencerCreationReceipt {
                    api_state: sequencer.api_state(),
                    endpoints,
                    background_handles,
                    proof_sender: Arc::new(sequencer),
                    api_ledger_db: api_ledger_db.clone(),
                    da_address,
                    is_replica: seq_role.is_replica(),
                })
            }
        }
    }

    /// Identical to [`FullNodeBlueprint::create_new_rollup`], but with
    /// a custom [`GenesisSource`].
    ///
    /// The genesis source is only consulted when the rollup state is empty;
    /// on a restart with populated state it is never read.
    #[tracing::instrument(name = "init_blueprint", skip_all)]
    async fn create_new_rollup_with_genesis_source(
        &self,
        genesis_source: GenesisSource<Self::Spec, Self::Runtime>,
        rollup_config: RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        prover_config: RollupProverConfig,
        start_at_rollup_height: Option<RollupHeight>,
        stop_at_rollup_height: Option<RollupHeight>,
        exec_config: Option<<<Self::Runtime as RuntimeTrait<Self::Spec>>::ModuleExecutionConfig as ModuleExecutionConfig>::Input>,
        start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<Rollup<Self, M>>
    where
        <Self::Spec as Spec>::Storage: NativeStorage,
    {
        if let Some(exec_config) = &exec_config {
            tracing::debug!("Initializing module execution config");
            <<Self::Runtime as RuntimeTrait<Self::Spec>>::ModuleExecutionConfig as ModuleExecutionConfig>::configure(exec_config).map_err(|e|anyhow::anyhow!(e))?;
        }

        let primary_shutdown = PrimaryShutdownController::new();
        let secondary_shutdown_controller = SecondaryShutdownController::new();
        let mut background_handles = vec![];

        let monitoring_config = rollup_config.monitoring.clone();
        if let Some(metrics_handle) =
            sov_metrics::init_metrics_tracker(&monitoring_config, &secondary_shutdown_controller)
        {
            background_handles.push(metrics_handle);
            background_handles.push(sov_metrics::spawn_tokio_runtime_metrics_task(
                std::time::Duration::from_millis(
                    monitoring_config.tokio_runtime_metrics_interval_millis,
                ),
                &secondary_shutdown_controller,
            ));
        } else {
            tracing::warn!("Metrics have been initialized outside of the rollup blueprint, some measurements can be lost on shutdown");
        };

        let da_service = self
            .create_da_service(&rollup_config, &secondary_shutdown_controller)
            .await;
        let da_service_handle = da_service.take_background_join_handle().await;
        if let Some(handle) = da_service_handle {
            background_handles.push(handle);
        }
        let da_service = Arc::new(da_service);

        macro_rules! startup_step {
            ($expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        cleanup_failed_startup_before_sequencer(
                            &primary_shutdown,
                            &secondary_shutdown_controller,
                            &mut background_handles,
                        )
                        .await;
                        return Err(error.into());
                    }
                }
            };
        }

        let da_polling_interval =
            std::time::Duration::from_millis(rollup_config.runner.da_polling_interval_ms);
        let da_service_with_cache = startup_step!(
            DaServiceWithCachedFinalizedHeaders::new(
                da_service.clone(),
                &secondary_shutdown_controller,
                da_polling_interval,
            )
            .await
        );
        let current_finalized_header =
            startup_step!(da_service.get_last_finalized_block_header().await);

        let witness_generation = prover_config.is_enabled();
        let mut storage_manager =
            startup_step!(self.create_storage_manager(&rollup_config, witness_generation));

        let (prover_storage, ledger_state) =
            startup_step!(storage_manager.create_state_after(&current_finalized_header));

        let ledger_db = startup_step!(self.create_ledger_db(ledger_state.clone()));
        // Create separate API LedgerDb that will be used to provide strong consistency for REST
        // API. The updating of the underlying ledger reader will be delayed until other components
        // have completed their processing.
        //
        // `ledger_db` will accumulate notifications by executing normally which will be
        // published by `api_ledger_db` at a time when the results are consistent with the
        // REST APIs view of the ledger.
        let api_ledger_db = LedgerDb::with_shared_notifications(&ledger_db);

        let prev_root = match startup_step!(ledger_db.get_head_slot()) {
            Some((number, _)) => Some(startup_step!(prover_storage
                .get_root_hash(number)
                .with_context(|| {
                    format!("missing root hash for committed head slot {number}")
                }))),
            None => None,
        };

        info!(
            ?prev_root,
            is_genesis = prev_root.is_none(),
            "Recovering the state root"
        );
        let native_stf = StfBlueprint::new();
        let mut rt = Self::Runtime::default();
        let (
            prover_storage,
            prev_state_root,
            genesis_state_root,
            operating_mode,
            genesis_da_height,
        ) = match prev_root {
            // Missing prev_root means need for initialization: obtain genesis
            // params now, deserializing them from files if needed.
            None => {
                let genesis_params = match genesis_source {
                    GenesisSource::Paths(paths) => {
                        startup_step!(self.create_genesis_config(&paths, &rollup_config))
                    }
                    GenesisSource::CustomParams(params) => params,
                };
                let operating_mode = <Self::Runtime as RuntimeTrait<Self::Spec>>::operating_mode(
                    &genesis_params.runtime,
                );
                startup_step!(validate_operating_mode_config(
                    operating_mode,
                    prover_config,
                    rollup_config.proof_manager.is_some(),
                ));
                let genesis_da_height = genesis_params.genesis_slot_number();
                info!(
                    rollup_genesis_height = genesis_da_height,
                    "Rollup state is empty, performing genesis initialization. Requesting genesis DA block"
                );
                let rollup_genesis_block =
                    startup_step!(da_service.get_block_at(genesis_da_height).await);

                let genesis_header = rollup_genesis_block.header().clone();
                let genesis_state_root: <<Self::Spec as Spec>::Storage as Storage>::Root = startup_step!(
                    initialize_state::<_, Self::DaService, _>(
                        &native_stf,
                        &mut storage_manager,
                        rollup_genesis_block,
                        genesis_params,
                    )
                    .await
                );

                // Re-create bootstrap storage, so it fetches the latest version after initialization.
                // And can see the latest changes. Otherwise Sequencer won't be able to process any batches,
                // because genesis data won't be visible to it.
                let (prover_storage, ledger_state) =
                    startup_step!(storage_manager.create_state_after(&genesis_header));

                ledger_db.replace_reader(ledger_state.clone());
                api_ledger_db.replace_reader(ledger_state);
                // Clearing notifications that has been produced during genesis.
                // Rollup is not running yet, so there are no subscribers.
                ledger_db.send_notifications();
                (
                    prover_storage,
                    genesis_state_root.clone(),
                    genesis_state_root,
                    operating_mode,
                    genesis_da_height,
                )
            }
            // LedgerDb contains previous state root, initialization already has been done.
            // The genesis source is intentionally not read: after a hard fork the on-disk
            // genesis files may not deserialize into the current `GenesisConfig`, and they
            // are not needed. The values genesis persisted to chain state are used instead.
            Some(prev_state_root) => {
                let genesis_state_root = startup_step!(prover_storage
                    .get_root_hash(SlotNumber::GENESIS)
                    .context("genesis root must exist when storage has prior state"));
                let mut checkpoint = StateCheckpoint::new(prover_storage.clone(), &rt.kernel());
                let operating_mode = rt.chain_state().operating_mode(&mut checkpoint);
                let genesis_da_height_result = {
                    let chain_state = rt.chain_state();
                    chain_state.genesis_da_height(&mut checkpoint).context(
                        "rollup state is initialized but `genesis_da_height` is missing from \
                         chain state; the database may be corrupted or produced by an \
                         incompatible binary",
                    )
                };
                let genesis_da_height = startup_step!(genesis_da_height_result);
                startup_step!(validate_operating_mode_config(
                    operating_mode,
                    prover_config,
                    rollup_config.proof_manager.is_some(),
                ));
                (
                    prover_storage,
                    prev_state_root,
                    genesis_state_root,
                    operating_mode,
                    genesis_da_height,
                )
            }
        };

        info!(?operating_mode, "Instantiating a new rollup");

        let da_sync_state = startup_step!(
            make_da_sync_state(
                genesis_da_height,
                stop_at_rollup_height,
                &ledger_db,
                &da_service_with_cache,
            )
            .await
        );

        let sync_status_receiver = da_sync_state.sync_status_sender.subscribe();

        let state_update_info = startup_step!(
            query_state_update_info(&ledger_db, prover_storage.clone(), da_sync_state.as_ref())
                .await
        );

        let checkpoint = StateCheckpoint::new(prover_storage, &rt.kernel());
        let current_height = checkpoint.rollup_height_to_access();

        startup_step!(validate_heights(
            current_height,
            start_at_rollup_height,
            stop_at_rollup_height,
        ));

        tracing::debug!(
            prev_root_hash = hex::encode(prev_state_root.as_ref()),
            raw_genesis_state_root = hex::encode(genesis_state_root.as_ref()),
            ?state_update_info,
            "Rollup state initialization is completed"
        );

        let state_channel = StateChannel::new(state_update_info);
        let state_update_receiver = state_channel.subscribe_state_update();
        let storage_receiver = state_channel.subscribe_storage();

        let visible_state_height_tracker: Box<dyn ProvableHeightTracker> = Box::new(
            MaximumProvableHeight::new(state_channel.subscribe_storage(), Self::Runtime::default()),
        );

        let axum_socket_addr = startup_step!(rollup_config.runner.http_config.socket_address());
        let axum_tcp = startup_step!(TcpListener::bind(axum_socket_addr).await);
        let axum_socket_addr = startup_step!(axum_tcp.local_addr());
        let mut sequencer = startup_step!(
            self.create_sequencer(
                state_update_receiver.clone(),
                da_sync_state.clone(),
                &rollup_config,
                &ledger_db,
                &api_ledger_db,
                &da_service,
                primary_shutdown.clone(),
                stop_at_rollup_height,
                axum_socket_addr,
            )
            .await
        );

        let proof_pipeline_enabled =
            should_enable_proof_pipeline(prover_config, sequencer.is_replica);

        // The prover service validates the latest aggregated proof persisted in
        // the ledger DB and returns its `final_slot_number`. We pass this slot
        // into the runner so the STF-info stream resumes at `final_slot + 1`.
        let (prover_service, latest_proof_final_slot) = if proof_pipeline_enabled {
            let create_prover_service_result = self
                .create_prover_service(
                    prover_config,
                    &rollup_config,
                    &da_service,
                    &ledger_db,
                    start_fresh_outer_proof_on_resync,
                )
                .await;

            match create_prover_service_result {
                Ok((svc, slot)) => (Some(svc), slot),
                Err(error) => {
                    cleanup_failed_startup(
                        &primary_shutdown,
                        &secondary_shutdown_controller,
                        &mut background_handles,
                        &mut sequencer,
                    )
                    .await;
                    return Err(error);
                }
            }
        } else {
            (None, None)
        };

        let runner_result = StateTransitionRunner::new(
            rollup_config.runner.clone(),
            axum_tcp,
            if proof_pipeline_enabled {
                rollup_config.proof_manager
            } else {
                None
            },
            da_service.clone(),
            ledger_db.clone(),
            native_stf,
            storage_manager,
            state_channel,
            prev_state_root,
            visible_state_height_tracker,
            primary_shutdown.clone(),
            start_at_rollup_height,
            stop_at_rollup_height,
            da_sync_state.clone(),
            da_service_with_cache,
            genesis_da_height,
            latest_proof_final_slot,
        )
        .await;
        let mut runner = match runner_result {
            Ok(runner) => runner,
            Err(error) => {
                cleanup_failed_startup(
                    &primary_shutdown,
                    &secondary_shutdown_controller,
                    &mut background_handles,
                    &mut sequencer,
                )
                .await;
                return Err(error);
            }
        };

        if let Some(stf_info_receiver) = runner.take_stf_info_receiver() {
            let prover_service = prover_service
                .expect("prover service must be present when stf_info_receiver is Some");
            let proof_sender: Box<dyn ProofSender> =
                match self.create_proof_sender(&rollup_config, sequencer.proof_sender.clone()) {
                    Ok(proof_sender) => Box::new(proof_sender),
                    Err(error) => {
                        let _ = runner.shutdown_before_run().await;
                        cleanup_failed_startup(
                            &primary_shutdown,
                            &secondary_shutdown_controller,
                            &mut background_handles,
                            &mut sequencer,
                        )
                        .await;
                        return Err(error);
                    }
                };
            let proof_manager = rollup_config
                .proof_manager
                .expect("proof_manager must be set when prover is enabled");

            let workflow_task_handle_result = match operating_mode {
                OperatingMode::Optimistic => {
                    let prover_address = proof_manager.prover_address;
                    let bonding_proof_service = Self::Runtime::default()
                        .proof_processor()
                        .create_bonding_proof_service::<Self::Runtime>(
                        prover_address,
                        storage_receiver,
                    );

                    start_op_workflow_in_background::<Self::ProverService, _>(
                        bonding_proof_service,
                        proof_sender,
                        &secondary_shutdown_controller,
                        stf_info_receiver,
                    )
                    .await
                }
                OperatingMode::Zk => {
                    start_zk_workflow_in_background(
                        prover_service,
                        proof_manager.aggregated_proof_block_jump,
                        proof_manager.eager_proof_submission,
                        proof_manager.max_number_of_aggregated_proofs_in_memory,
                        proof_sender,
                        stf_info_receiver,
                        runner.da_sync_state(),
                        &secondary_shutdown_controller,
                        primary_shutdown.clone(),
                        start_fresh_outer_proof_on_resync,
                    )
                    .await
                }
                OperatingMode::Operator => {
                    Ok(start_operator_workflow_in_background(&secondary_shutdown_controller).await)
                }
            };
            let workflow_task_handle = match workflow_task_handle_result {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = runner.shutdown_before_run().await;
                    cleanup_failed_startup(
                        &primary_shutdown,
                        &secondary_shutdown_controller,
                        &mut background_handles,
                        &mut sequencer,
                    )
                    .await;
                    return Err(error);
                }
            };

            background_handles.push(workflow_task_handle);
        }

        let endpoints_result = self
            .create_endpoints(
                state_update_receiver,
                sync_status_receiver,
                primary_shutdown.clone(),
                &api_ledger_db,
                &sequencer,
                &da_service,
                &rollup_config,
            )
            .await;
        let endpoints = match endpoints_result {
            Ok(endpoints) => endpoints,
            Err(error) => {
                let _ = runner.shutdown_before_run().await;
                cleanup_failed_startup(
                    &primary_shutdown,
                    &secondary_shutdown_controller,
                    &mut background_handles,
                    &mut sequencer,
                )
                .await;
                return Err(error);
            }
        };

        let endpoints = NodeEndpointsContainer {
            inner: endpoints,
            cors_configuration: rollup_config.runner.http_config.cors,
        };

        background_handles.extend(sequencer.background_handles);

        spawn_os_signal_handler(primary_shutdown.clone());

        Ok(Rollup {
            runner,
            endpoints,
            primary_shutdown,
            secondary_shutdown_controller,
            background_handles,
            genesis_slot_number: genesis_da_height,
            rpc_aggregation_config: monitoring_config.rpc_aggregation.clone(),
        })
    }
}

fn validate_operating_mode_config(
    operating_mode: OperatingMode,
    prover_config: RollupProverConfig,
    proof_manager_configured: bool,
) -> anyhow::Result<()> {
    if operating_mode == OperatingMode::Operator && prover_config.is_enabled() {
        anyhow::bail!(
            "The operating mode is set to `{operating_mode:?}` and prover config is set to `{prover_config:?}`. This is not supported",
        );
    }

    if operating_mode != OperatingMode::Operator && !proof_manager_configured {
        anyhow::bail!(
            "Missing `[proof_manager]` section in rollup config: it is required for `{operating_mode:?}` rollups.",
        );
    }

    Ok(())
}

async fn cleanup_failed_startup<S: Spec>(
    primary_shutdown: &PrimaryShutdownController,
    secondary_shutdown_controller: &SecondaryShutdownController,
    background_handles: &mut Vec<JoinHandle<()>>,
    sequencer: &mut SequencerCreationReceipt<S>,
) {
    primary_shutdown.shutdown();
    secondary_shutdown_controller.shutdown();

    let background_handles_to_join = std::mem::take(background_handles);
    let sequencer_background_handles = std::mem::take(&mut sequencer.background_handles);
    let endpoint_background_handles = std::mem::take(&mut sequencer.endpoints.background_handles);

    wait_for_failed_startup_tasks(
        background_handles_to_join,
        sequencer_background_handles,
        endpoint_background_handles,
    )
    .await;
}

async fn cleanup_failed_startup_before_sequencer(
    primary_shutdown: &PrimaryShutdownController,
    secondary_shutdown_controller: &SecondaryShutdownController,
    background_handles: &mut Vec<JoinHandle<()>>,
) {
    primary_shutdown.shutdown();
    secondary_shutdown_controller.shutdown();

    let background_handles_to_join = std::mem::take(background_handles);

    wait_for_failed_startup_tasks(background_handles_to_join, Vec::new(), Vec::new()).await;
}

async fn wait_for_failed_startup_tasks(
    background_handles_to_join: Vec<JoinHandle<()>>,
    sequencer_background_handles: Vec<JoinHandle<()>>,
    endpoint_background_handles: Vec<JoinHandle<anyhow::Result<()>>>,
) {
    // Drain handles concurrently rather than serially.
    let drain = async move {
        let _ = tokio::join!(
            future::join_all(background_handles_to_join),
            future::join_all(sequencer_background_handles),
            future::join_all(endpoint_background_handles),
        );
    };

    if tokio::time::timeout(std::time::Duration::from_secs(30), drain)
        .await
        .is_err()
    {
        tracing::warn!(
            "Timed out waiting for background tasks to drain after a failed startup; \
             some tasks may still hold storage references"
        );
    }
}

fn validate_heights(
    current_height: RollupHeight,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
) -> anyhow::Result<()> {
    if let Some(start_at_rollup_height) = start_at_rollup_height {
        let expected_start_at_rollup_height = current_height
            .checked_add(1)
            .expect("Height calculation overflow");
        if start_at_rollup_height != expected_start_at_rollup_height {
            anyhow::bail!(
                "The requested start_at_rollup_height: {start_at_rollup_height}, is different than expected height {expected_start_at_rollup_height}"
            );
        }
    }

    if let Some(stop_height) = stop_at_rollup_height {
        if stop_height <= current_height {
            tracing::error!(
                stop_height = stop_height.get(),
                rollup_height_to_access = current_height.get(),
                "The requested stop_height must be greater than the current rollup_height_to_access"
            );
            anyhow::bail!("The requested stop_height {stop_height} must be greater than the current_height {current_height}");
        }
    }

    Ok(())
}

/// [`NodeEndpoints`] with `CORS` configuration.
pub struct NodeEndpointsContainer {
    inner: NodeEndpoints,
    cors_configuration: CorsConfiguration,
}

/// Dependencies needed to run the rollup.
pub struct Rollup<S: FullNodeBlueprint<M>, M: ExecutionMode> {
    /// The State Transition Runner.
    #[allow(clippy::type_complexity)]
    pub runner:
        StateTransitionRunner<StfBlueprint<S::Spec, S::Runtime>, S::StorageManager, S::DaService>,

    /// Server endpoints for the rollup.
    pub endpoints: NodeEndpointsContainer,

    /// A way to gracefully shut down background tasks.
    pub primary_shutdown: PrimaryShutdownController,

    /// The genesis slot number.
    pub genesis_slot_number: u64,

    // Trigger after the runner has finished.
    secondary_shutdown_controller: SecondaryShutdownController,

    background_handles: Vec<tokio::task::JoinHandle<()>>,

    /// RPC metrics aggregation settings, captured from
    /// `rollup_config.monitoring` at creation time and handed to the HTTP
    /// server in [`Rollup::run`].
    rpc_aggregation_config: sov_metrics::RpcAggregationConfig,
}

impl<S: FullNodeBlueprint<M>, M: ExecutionMode> Rollup<S, M> {
    /// Runs the rollup.
    pub async fn run(self) -> anyhow::Result<()> {
        let mut runner = self.runner;

        runner
            .start_http_server(
                self.endpoints.inner.axum_router,
                self.endpoints.inner.jsonrpsee_module,
                self.endpoints.cors_configuration,
                self.rpc_aggregation_config,
            )
            .await
            .context("Failed to start Axum Server")?;

        let monitoring_task =
            spawn_task_monitor(self.primary_shutdown.clone(), self.background_handles);

        runner.run_in_process().await?;
        tracing::info!("STF Runner has completed execution");

        self.primary_shutdown.shutdown();

        self.secondary_shutdown_controller.shutdown();

        // blocks until background handles have shutdown
        monitoring_task.await??;
        for handle in self.endpoints.inner.background_handles {
            match handle.await {
                Err(e) => {
                    tracing::error!(error = %e, "Endpoint background task panicked.");
                    return Err(e.into());
                }
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "Endpoint background task joined with error");
                    return Err(e);
                }
                _ => {}
            }
        }
        tracing::debug!("Rollup completed run");
        Ok(())
    }
}

fn spawn_task_monitor(
    primary_shutdown: PrimaryShutdownController,
    handles: Vec<tokio::task::JoinHandle<()>>,
) -> tokio::task::JoinHandle<Result<(), anyhow::Error>> {
    tokio::spawn(async move {
        tracing::trace!("blocking until a background task joins or rollup shutdown");
        let (result, _, handles) = futures::future::select_all(handles).await;

        let mut was_graceful = if let Err(error) = result {
            tracing::error!(error = %error, "background task joined with error");
            primary_shutdown.shutdown();
            false
        } else {
            // If no shutdown has been triggered then it's implied that one of the handles
            // joined early before a shutdown signal was sent. This likely indicates
            // incorrect behaviour and so we send the signal ourselves to begin the shutdown process.
            if primary_shutdown.is_triggered() {
                true
            } else {
                tracing::error!("background task joined with success status but no shutdown signal had been sent at the time. This is a bug! Please report it.");
                // Start graceful shutdown
                primary_shutdown.shutdown();
                false
            }
        };

        tracing::trace!("waiting for background tasks to join");

        for handle in handles {
            if let Err(error) = handle.await {
                tracing::error!(error = %error, "Additional background task joined with error");
                primary_shutdown.shutdown();
                was_graceful = false;
            }
        }

        tracing::trace!("task monitoring is complete");
        if !was_graceful {
            anyhow::bail!("One or more background tasks joined with errors. See logs for details.");
        }

        Ok(())
    })
}

fn spawn_os_signal_handler(primary_shutdown: PrimaryShutdownController) {
    tokio::spawn(async move {
        let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
            .expect("Failed to set up SIGTERM handler");
        let mut quit = tokio::signal::unix::signal(SignalKind::quit())
            .expect("Failed to set up SIGQUIT handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
            _ = terminate.recv() => tracing::info!("Received SIGTERM"),
            _ = quit.recv() => tracing::info!("Received SIGQUIT"),
            _ = primary_shutdown.wait_for_shutdown() => {
                tracing::debug!("Stopping OS signal handling task, as rollup has been stopped programmatically");
                return;
            }
        }
        primary_shutdown.shutdown();
    });
}

/// The result of [`FullNodeBlueprint::create_sequencer`].
pub struct SequencerCreationReceipt<S: Spec> {
    /// The [`ApiState`] that shall be used by REST APIs.
    ///
    /// See [`sov_modules_api::rest::HasRestApi::rest_api`].
    pub api_state: ApiState<S>,
    /// Will be passed to [`FullNodeBlueprint::create_proof_sender`].
    ///
    /// See [`crate::proof_sender::SovApiProofSender::new`].
    pub proof_sender: Arc<dyn ProofBlobSender>,
    /// The API LedgerDb that the sequencer will update for REST API consistency
    pub api_ledger_db: LedgerDb,
    #[allow(missing_docs)]
    pub endpoints: NodeEndpoints,
    #[allow(missing_docs)]
    pub background_handles: Vec<JoinHandle<()>>,
    #[allow(missing_docs)]
    pub da_address: <S::Da as DaSpec>::Address,
    /// Whether the resolved sequencer role is a replica role.
    pub is_replica: bool,
}

fn should_enable_proof_pipeline(prover_config: RollupProverConfig, is_replica: bool) -> bool {
    prover_config.is_enabled() && !is_replica
}
