mod endpoints;
pub mod logging;
pub mod proof_sender;
mod telemetry;
mod wallet;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
pub use endpoints::*;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::{DeltaReader, SchemaBatch};
use sov_modules_api::capabilities::{HasCapabilities, HasKernel, ProofProcessor, RollupHeight};
use sov_modules_api::execution_mode::ExecutionMode;
use sov_modules_api::provable_height_tracker::MaximumProvableHeight;
use sov_modules_api::rest::{ApiState, StateUpdateReceiver};
use sov_modules_api::{
    DaSpec, NodeEndpoints, OperatingMode, ProofSender, Spec, StateCheckpoint, StateUpdateInfo,
    SyncStatus, VersionReader, ZkVerifier,
};
use sov_modules_stf_blueprint::{GenesisParams, Runtime as RuntimeTrait, StfBlueprint};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::DaSyncState;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::ProvableHeightTracker;
use sov_sequencer::preferred::PreferredSequencer;
use sov_sequencer::standard::StdSequencer;
use sov_sequencer::{ProofBlobSender, Sequencer, SequencerApis, SequencerKindConfig};
use sov_state::storage::NativeStorage;
use sov_state::Storage;
use sov_stf_runner::processes::{
    start_op_workflow_in_background, start_operator_workflow_in_background,
    start_zk_workflow_in_background, ProverService, RollupProverConfig,
    RollupProverConfigDiscriminants,
};
use sov_stf_runner::{
    initialize_state, query_state_update_info, CorsConfiguration, RollupConfig,
    StateTransitionRunner,
};
use tokio::signal::unix::SignalKind;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::info;
pub use wallet::*;

use crate::RollupBlueprint;

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

    /// Creates code commitments for the outer zkVM program.
    fn create_outer_code_commitment(
        &self,
    ) -> <<Self::ProverService as ProverService>::Verifier as ZkVerifier>::CodeCommitment;

    /// Creates RPC methods and REST APIs for the rollup.
    async fn create_endpoints(
        &self,
        state_update_receiver: StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        sync_status_receiver: tokio::sync::watch::Receiver<SyncStatus>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
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
        shutdown_receiver: watch::Receiver<()>,
    ) -> Self::DaService;

    /// Creates an instance of [`ProverService`].
    async fn create_prover_service(
        &self,
        prover_config: RollupProverConfig<<Self::Spec as Spec>::InnerZkvm>,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        da_service: &Self::DaService,
    ) -> Self::ProverService;

    /// Creates an instance of [`Self::StorageManager`].
    /// Panics if initialization fails.
    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<Self::StorageManager>;

    /// Instantiates [`FullNodeBlueprint::ProofSender`].
    fn create_proof_sender(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        sequencer: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender>;

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
        prover_config: Option<RollupProverConfig<<Self::Spec as Spec>::InnerZkvm>>,
        start_at_rollup_height: Option<RollupHeight>,
        stop_at_rollup_height: Option<RollupHeight>,
    ) -> anyhow::Result<Rollup<Self, M>>
    where
        <Self::Spec as Spec>::Storage: NativeStorage,
    {
        let genesis_params = self.create_genesis_config(runtime_genesis_paths, &rollup_config)?;

        self.create_new_rollup_with_genesis_params(
            genesis_params,
            rollup_config,
            prover_config,
            start_at_rollup_height,
            stop_at_rollup_height,
        )
        .await
    }

    /// Injects additional HTTP APIs for the sequencer.
    async fn sequencer_additional_apis<Seq>(
        &self,
        _sequencer: Arc<Seq>,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        Ok(NodeEndpoints::default())
    }

    /// Creates a new sequencer and provides a [`SequencerCreationReceipt`] with
    /// some information about said sequencer.
    async fn create_sequencer(
        &self,
        state_update_receiver: watch::Receiver<StateUpdateInfo<<Self::Spec as Spec>::Storage>>,
        da_sync_state: Arc<DaSyncState>,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        ledger_db: &LedgerDb,
        api_ledger_db: &LedgerDb,
        da_service: &Self::DaService,
        shutdown_receiver: watch::Receiver<()>,
        shutdown_sender: tokio::sync::watch::Sender<()>,
        stop_at_rollup_height: Option<RollupHeight>,
    ) -> anyhow::Result<SequencerCreationReceipt<Self::Spec>> {
        match &rollup_config.sequencer.sequencer_kind_config {
            SequencerKindConfig::Standard(seq_config) => {
                let (sequencer, background_handles) =
                    StdSequencer::<Self::Spec, Self::Runtime, Self::DaService>::create(
                        da_service.clone(),
                        state_update_receiver.clone(),
                        da_sync_state,
                        &rollup_config.storage.path,
                        &rollup_config.sequencer.with_seq_config(seq_config.clone()),
                        ledger_db.clone(),
                        api_ledger_db.clone(),
                        shutdown_sender,
                    )
                    .await?;

                let mut endpoints = self
                    .sequencer_additional_apis(sequencer.clone(), rollup_config)
                    .await?;
                endpoints.axum_router = endpoints.axum_router.merge(
                    SequencerApis::rest_api_server(sequencer.clone(), shutdown_receiver),
                );

                Ok(SequencerCreationReceipt {
                    api_state: sequencer.api_state(),
                    endpoints,
                    background_handles,
                    proof_sender: sequencer,
                    api_ledger_db: api_ledger_db.clone(),
                    da_address: da_service.get_signer().await,
                })
            }
            SequencerKindConfig::Preferred(seq_config) => {
                let (sequencer, background_handles) =
                    PreferredSequencer::<Self::Spec, Self::Runtime, Self::DaService>::create(
                        da_service.clone(),
                        state_update_receiver.clone(),
                        da_sync_state,
                        &rollup_config.storage.path,
                        &rollup_config.sequencer.with_seq_config(seq_config.clone()),
                        ledger_db.clone(),
                        api_ledger_db.clone(),
                        shutdown_sender.clone(),
                        stop_at_rollup_height,
                    )
                    .await?;

                let mut endpoints = self
                    .sequencer_additional_apis(sequencer.clone(), rollup_config)
                    .await?;
                endpoints.axum_router = endpoints.axum_router.merge(
                    SequencerApis::rest_api_server(sequencer.clone(), shutdown_receiver),
                );

                Ok(SequencerCreationReceipt {
                    api_state: sequencer.api_state(),
                    endpoints,
                    background_handles,
                    proof_sender: sequencer,
                    api_ledger_db: api_ledger_db.clone(),
                    da_address: da_service.get_signer().await,
                })
            }
        }
    }

    /// Identical to [`FullNodeBlueprint::create_new_rollup`], but with
    /// a custom [`GenesisParams`].
    #[tracing::instrument(name = "init_blueprint", skip_all)]
    async fn create_new_rollup_with_genesis_params(
        &self,
        genesis_params: GenesisParams<<Self::Runtime as RuntimeTrait<Self::Spec>>::GenesisConfig>,
        rollup_config: RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        prover_config: Option<RollupProverConfig<<Self::Spec as Spec>::InnerZkvm>>,
        start_at_rollup_height: Option<RollupHeight>,
        stop_at_rollup_height: Option<RollupHeight>,
    ) -> anyhow::Result<Rollup<Self, M>>
    where
        <Self::Spec as Spec>::Storage: NativeStorage,
    {
        let (main_shutdown_sender, mut main_shutdown_receiver) = tokio::sync::watch::channel(());
        main_shutdown_receiver.mark_unchanged();
        let (secondary_shutdown_sender, mut secondary_shutdown_receiver) =
            tokio::sync::watch::channel(());
        secondary_shutdown_receiver.mark_unchanged();

        let operating_mode =
            <Self::Runtime as RuntimeTrait<Self::Spec>>::operating_mode(&genesis_params.runtime);
        info!(?operating_mode, "Instantiating a new rollup");

        if let (OperatingMode::Operator, Some(prover_config)) =
            (operating_mode, prover_config.clone())
        {
            let prover_config: RollupProverConfigDiscriminants = prover_config.into();
            panic!("The operating mode is set to `{operating_mode:?}` and prover config is set to `{prover_config:?}`. This is not supported");
        }

        let da_service = self
            .create_da_service(&rollup_config, secondary_shutdown_receiver.clone())
            .await;
        let da_service_handle = da_service.take_background_join_handle().await;
        let da_service = Arc::new(da_service);
        let current_finalized_header = da_service.get_last_finalized_block_header().await?;

        let mut storage_manager = self.create_storage_manager(&rollup_config)?;

        let (prover_storage, ledger_state) =
            storage_manager.create_state_after(&current_finalized_header)?;

        let ledger_db = self.create_ledger_db(ledger_state.clone())?;
        // Create separate API LedgerDb that will be used to provide strong consistency for REST
        // API. The updating of the underlying ledger reader will be delayed until other components
        // have completed their processing.
        //
        // `ledger_db` will accumulate notifications by executing normally which will be
        // published by `api_ledger_db` at a time when the results are consistent with the
        // REST APIs view of the ledger.
        let api_ledger_db = LedgerDb::with_shared_notifications(&ledger_db);

        let prev_root = ledger_db
            .get_head_slot()?
            .map(|(number, _)| prover_storage.get_root_hash(number))
            .transpose()?;

        info!(
            ?prev_root,
            is_genesis = prev_root.is_none(),
            "Recovering the state root"
        );
        let native_stf = StfBlueprint::new();
        let (prover_storage, prev_state_root, genesis_state_root) = match prev_root {
            // Missing prev_root means need for initialization
            None => {
                info!(
                    rollup_genesis_height = rollup_config.runner.genesis_height,
                    "Rollup state is empty, performing genesis initialization. Requesting genesis DA block"
                );
                let rollup_genesis_block = da_service
                    .get_block_at(rollup_config.runner.genesis_height)
                    .await?;

                let genesis_header = rollup_genesis_block.header().clone();
                let genesis_state_root: <<Self::Spec as Spec>::Storage as Storage>::Root =
                    initialize_state::<_, _, _, Self::DaService, _>(
                        &native_stf,
                        &mut storage_manager,
                        rollup_genesis_block,
                        genesis_params,
                    )
                    .await?;

                // Re-create bootstrap storage, so it fetches the latest version after initialization.
                // And can see the latest changes. Otherwise Sequencer won't be able to process any batches,
                // because genesis data won't be visible to it.
                let (prover_storage, ledger_state) =
                    storage_manager.create_state_after(&genesis_header)?;

                ledger_db.replace_reader(ledger_state.clone());
                api_ledger_db.replace_reader(ledger_state);
                // Clearing notifications that has been produced during genesis.
                // Rollup is not running yet, so there are no subscribers.
                ledger_db.send_notifications();
                (
                    prover_storage,
                    genesis_state_root.clone(),
                    genesis_state_root,
                )
            }
            // LedgerDb contains previous state root, initialization already has been done.
            Some(prev_state_root) => {
                let genesis_state_root = prover_storage.get_root_hash(SlotNumber::GENESIS)?;
                // (prev_state_root, genesis_state_root)
                (prover_storage, prev_state_root, genesis_state_root)
            }
        };

        let state_update_info = query_state_update_info(&ledger_db, prover_storage.clone()).await?;

        let mut rt = Self::Runtime::default();
        let checkpoint = StateCheckpoint::new(prover_storage, &rt.kernel());
        let current_height = checkpoint.rollup_height_to_access();

        validate_heights(
            current_height,
            start_at_rollup_height,
            stop_at_rollup_height,
        )?;

        tracing::debug!(
            prev_root_hash = hex::encode(prev_state_root.as_ref()),
            raw_genesis_state_root = hex::encode(genesis_state_root.as_ref()),
            ?state_update_info,
            "Rollup state initialization is completed"
        );

        let (state_update_sender, state_update_receiver) =
            tokio::sync::watch::channel(state_update_info);

        let mut background_handles = vec![];
        if let Some(handle) = da_service_handle {
            background_handles.push(handle);
        }

        let visible_state_height_tracker: Box<dyn ProvableHeightTracker> = Box::new(
            MaximumProvableHeight::new(state_update_sender.subscribe(), Self::Runtime::default()),
        );

        let (sync_status_sender, sync_status_receiver) =
            tokio::sync::watch::channel(SyncStatus::START);

        let mut runner = StateTransitionRunner::new(
            rollup_config.runner.clone(),
            if prover_config.is_some() {
                Some(rollup_config.proof_manager.clone())
            } else {
                None
            },
            da_service.clone(),
            ledger_db.clone(),
            native_stf,
            storage_manager,
            state_update_sender,
            prev_state_root,
            sync_status_sender,
            visible_state_height_tracker,
            main_shutdown_receiver.clone(),
            rollup_config.monitoring.clone(),
            start_at_rollup_height,
            stop_at_rollup_height,
        )
        .await?;

        let sequencer = self
            .create_sequencer(
                state_update_receiver.clone(),
                runner.da_sync_state(),
                &rollup_config,
                &ledger_db,
                &api_ledger_db,
                &da_service,
                main_shutdown_receiver.clone(),
                main_shutdown_sender.clone(),
                stop_at_rollup_height,
            )
            .await?;

        if let Some(stf_info_receiver) = runner.take_stf_info_receiver() {
            let prover_config = prover_config
                .expect("This code path should not be possible; this is a bug, please report it");

            let prover_service = self
                .create_prover_service(prover_config, &rollup_config, &da_service)
                .await;

            let proof_sender =
                Box::new(self.create_proof_sender(&rollup_config, sequencer.proof_sender.clone())?);

            let workflow_task_handle = match operating_mode {
                OperatingMode::Optimistic => {
                    let prover_address = rollup_config.proof_manager.prover_address.clone();
                    let bonding_proof_service = Self::Runtime::default()
                        .proof_processor()
                        .create_bonding_proof_service::<Self::Runtime>(
                        prover_address,
                        state_update_receiver.clone(),
                    );

                    start_op_workflow_in_background::<Self::ProverService, _>(
                        bonding_proof_service,
                        proof_sender,
                        secondary_shutdown_receiver,
                        stf_info_receiver,
                    )
                    .await?
                }
                OperatingMode::Zk => {
                    start_zk_workflow_in_background(
                        prover_service,
                        rollup_config.proof_manager.aggregated_proof_block_jump,
                        proof_sender,
                        genesis_state_root,
                        stf_info_receiver,
                        secondary_shutdown_receiver,
                    )
                    .await?
                }
                OperatingMode::Operator => {
                    start_operator_workflow_in_background(secondary_shutdown_receiver).await
                }
            };

            background_handles.push(workflow_task_handle);
        }

        let endpoints = self
            .create_endpoints(
                state_update_receiver,
                sync_status_receiver,
                main_shutdown_receiver.clone(),
                &api_ledger_db,
                &sequencer,
                &da_service,
                &rollup_config,
            )
            .await?;

        let endpoints = NodeEndpointsContainer {
            inner: endpoints,
            cors_configuration: rollup_config.runner.http_config.cors,
        };

        background_handles.extend(sequencer.background_handles);

        spawn_os_signal_handler(main_shutdown_sender.clone());

        Ok(Rollup {
            runner,
            endpoints,
            shutdown_sender: main_shutdown_sender,
            secondary_shutdown_sender,
            background_handles,
        })
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
    pub runner: StateTransitionRunner<
        StfBlueprint<S::Spec, S::Runtime>,
        S::StorageManager,
        S::DaService,
        <S::Spec as Spec>::InnerZkvm,
        <S::Spec as Spec>::OuterZkvm,
    >,

    /// Server endpoints for the rollup.
    pub endpoints: NodeEndpointsContainer,

    /// A way to gracefully shut down background tasks.
    pub shutdown_sender: tokio::sync::watch::Sender<()>,

    // Trigger after the runner has finished.
    secondary_shutdown_sender: tokio::sync::watch::Sender<()>,

    background_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl<S: FullNodeBlueprint<M>, M: ExecutionMode> Rollup<S, M> {
    /// Runs the rollup.
    pub async fn run(self) -> anyhow::Result<()> {
        self.run_and_report_addr(None).await
    }

    /// Runs the rollup. Reports REST and RPC ports to the caller using the provided channel.
    pub async fn run_and_report_addr(
        self,
        axum_addr_channel: Option<oneshot::Sender<SocketAddr>>,
    ) -> anyhow::Result<()> {
        let mut runner = self.runner;

        let axum_addr = runner
            .start_http_server(
                self.endpoints.inner.axum_router,
                self.endpoints.inner.jsonrpsee_module,
                self.endpoints.cors_configuration,
            )
            .await
            .context("Failed to start Axum Server")?;
        if let Some(sender) = axum_addr_channel {
            sender
                .send(axum_addr)
                .map_err(|_| anyhow::anyhow!("Failed to send Axum address"))?;
        }

        let monitoring_task =
            spawn_task_monitor(self.shutdown_sender.clone(), self.background_handles);

        runner.run_in_process().await?;
        tracing::info!("STF Runner has completed execution");

        if self.shutdown_sender.send(()).is_err() {
            tracing::info!(
                "Failed to send primary shutdown signal because all receivers have been dropped"
            );
        }

        if self.secondary_shutdown_sender.send(()).is_err() {
            tracing::info!(
                "Failed to send secondary shutdown signal because all receivers have been dropped"
            );
        }

        // blocks until background handles have shutdown
        monitoring_task.await??;
        for handle in self.endpoints.inner.background_handles {
            handle.await??;
        }
        tracing::debug!("Rollup completed run");
        Ok(())
    }
}

fn spawn_task_monitor(
    shutdown_sender: tokio::sync::watch::Sender<()>,
    handles: Vec<tokio::task::JoinHandle<()>>,
) -> tokio::task::JoinHandle<Result<(), tokio::task::JoinError>> {
    tokio::spawn(async move {
        let shutdown_recv = shutdown_sender.subscribe();
        tracing::trace!("blocking until a background task joins or rollup shutdown");
        let (result, _, handles) = futures::future::select_all(handles).await;

        if let Err(error) = result {
            tracing::error!(error = %error, "background task joined with error");
        } else {
            // If shutdown receiver hasn't changed then it's implied that one of the handles
            // joined early before a shutdown signal was sent. This likely indicates
            // incorrect behaviour and so we send the signal ourselves to begin the shutdown process.
            if let Ok(false) = shutdown_recv.has_changed() {
                tracing::error!("background task joined with success status and no shutdown signal was sent, this is a error!");
                // Start graceful shutdown
                _ = shutdown_sender.send(());
            }
        }

        tracing::trace!("waiting for background tasks to join");

        for handle in handles {
            handle.await?;
        }

        tracing::trace!("task monitoring is complete");

        Ok(())
    })
}

fn spawn_os_signal_handler(shutdown_sender: tokio::sync::watch::Sender<()>) {
    tokio::spawn(async move {
        let mut api_shutdown = shutdown_sender.subscribe();
        let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
            .expect("Failed to set up SIGTERM handler");
        let mut quit = tokio::signal::unix::signal(SignalKind::quit())
            .expect("Failed to set up SIGQUIT handler");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
            _ = terminate.recv() => tracing::info!("Received SIGTERM"),
            _ = quit.recv() => tracing::info!("Received SIGQUIT"),
            _ = api_shutdown.changed() => {
                tracing::debug!("Stopping OS signal handling task, as rollup has been stopped programmatically");
                return;
            }
        }
        shutdown_sender
            .send(())
            .expect("Failed to send shutdown signal");
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
}
