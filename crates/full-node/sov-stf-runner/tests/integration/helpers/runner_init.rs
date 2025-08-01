use std::num::NonZero;
use std::sync::Arc;

use axum::async_trait;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use rockbound::SchemaBatch;
use sha2::Sha256;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::DeltaReader;
use sov_db::storage_manager::NativeStorageManager;
use sov_metrics::MonitoringConfig;
use sov_mock_da::{
    BlockProducingConfig, MockAddress, MockBlockHeader, MockDaConfig, MockDaService, MockDaSpec,
    MockDaVerifier, MockHash,
};
use sov_mock_zkvm::{MockZkvm, MockZkvmHost};
use sov_modules_api::provable_height_tracker::InfiniteHeight;
use sov_modules_api::{
    FullyBakedTx, ProofSender, StateTransitionFunction, StateUpdateInfo, SyncStatus,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::ledger_api::{AggregatedProofResponse, LedgerStateProvider};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::Zkvm;
use sov_sequencer::standard::StdSequencerConfig;
use sov_sequencer::{react_to_state_updates, SequencerConfig, SequencerKindConfig};
use sov_state::{DefaultStorageSpec, NativeStorage, ProverStorage};
use sov_stf_runner::processes::{
    start_zk_workflow_in_background, ParallelProverService, RollupProverConfigDiscriminants,
};
use sov_stf_runner::{
    initialize_state, query_state_update_info, HttpServerConfig, ProofManagerConfig, RollupConfig,
    RunnerConfig, StateTransitionRunner,
};
use sov_test_utils::{
    TestSpec, TEST_BLOB_PROCESSING_TIMEOUT, TEST_MAX_BATCH_SIZE, TEST_MAX_CONCURRENT_BLOBS,
};
use tokio::sync::broadcast::Receiver;
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::helpers::hash_stf::HashStf;

type MockInitVariant = InitVariant<HashStf, MockZkvm, MockZkvm, MockDaService>;

pub type S = DefaultStorageSpec<Sha256>;
pub type StorageManager = NativeStorageManager<MockDaSpec, ProverStorage<S>>;
pub type HashStfRunner<Da> = StateTransitionRunner<HashStf, StorageManager, Da, MockZkvm, MockZkvm>;

/// TestNode simulates a full-node.
pub struct TestNode {
    proof_posted_in_da_sub: Receiver<()>,
    agg_proof_saved_in_db_sub: BoxStream<'static, AggregatedProofResponse>,
    da: Arc<MockDaService>,
    inner_vm: MockZkvmHost,
    _outer_vm: MockZkvmHost,
    tasks: JoinSet<()>,
    // Just to remove warnings from logs
    _sync_status_receiver: watch::Receiver<SyncStatus>,
    shutdown_sender: watch::Sender<()>,
}

impl TestNode {
    /// Creates a DA block containing a transaction blob, optionally including an aggregated proof.
    pub async fn send_transaction(&self) -> anyhow::Result<MockHash> {
        let batch = vec![FullyBakedTx {
            data: vec![1, 2, 3],
        }];

        let serialized_batch = borsh::to_vec(&batch)?;
        self.da
            .send_transaction(&serialized_batch)
            .await
            .await?
            .map(|receipt| receipt.da_transaction_id)
    }

    /// Creates a DA block containing an empty transaction blob, optionally including an aggregated proof.
    pub async fn try_send_aggregated_proof(&self) -> anyhow::Result<MockHash> {
        let batch = vec![FullyBakedTx { data: vec![] }];
        let serialized_batch = borsh::to_vec(&batch)?;
        self.da
            .send_transaction(&serialized_batch)
            .await
            .await?
            .map(|receipt| receipt.da_transaction_id)
    }

    /// Unlocks the prover service worker thread and completes the block proof.
    pub fn make_block_proof(&self) {
        self.inner_vm.make_proof();
    }

    /// The aggregated proof was posted to DA and will be included in the NEXT block.
    pub async fn wait_for_aggregated_proof_posted_to_da(&mut self) -> anyhow::Result<()> {
        Ok(self.proof_posted_in_da_sub.recv().await?)
    }

    /// The aggregated proof was saved in the db.
    pub async fn wait_for_aggregated_proof_saved_in_db(&mut self) -> AggregatedProofResponse {
        self.agg_proof_saved_in_db_sub
            .next()
            .await
            .expect("No more aggregated proofs; this is a bug, please report it")
    }

    pub async fn stop(self) {
        self.shutdown_sender.send(()).unwrap();
        let _ = self.tasks.join_all().await;
    }
}

struct MockProofSender {
    da: Arc<MockDaService>,
}

#[async_trait]
impl ProofSender for MockProofSender {
    async fn publish_proof_blob_with_metadata(
        &self,
        serialized_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        let serialized_blob = serialized_proof.raw_aggregated_proof;

        self.da.send_proof(&serialized_blob).await.await??;

        Ok(())
    }

    async fn publish_attestation_blob_with_metadata(
        &self,
        _serialized_attestation: sov_modules_api::SerializedAttestation,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }

    async fn publish_challenge_blob_with_metadata(
        &self,
        _serialized_challenge: sov_modules_api::SerializedChallenge,
        _slot_height: SlotNumber,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
}

// Returns genesis state root, prev state root for given init variant and initial value for state update info.
pub async fn bootstrap_state_update_info(
    storage_manager: &mut StorageManager,
) -> anyhow::Result<StateUpdateInfo<ProverStorage<S>>> {
    let genesis_block_header = MockBlockHeader::from_height(0);
    let (stf_storage, ledger_state) = storage_manager.create_state_after(&genesis_block_header)?;
    let ledger_db = LedgerDb::with_reader(ledger_state)?;

    query_state_update_info(&ledger_db, stf_storage).await
}

pub async fn initialize_runner(
    da_service: Arc<MockDaService>,
    path: &std::path::Path,
    init_variant: MockInitVariant,
    aggregated_proof_block_jump: usize,
    nb_of_prover_threads: Option<usize>,
) -> (HashStfRunner<MockDaService>, TestNode) {
    let stf = HashStf::new();
    let inner_vm = MockZkvmHost::new();
    let outer_vm = MockZkvmHost::new_non_blocking();
    let verifier = MockDaVerifier::default();

    let rollup_config = rollup_config(&da_service, path, aggregated_proof_block_jump);
    let mut storage_manager: StorageManager = NativeStorageManager::new(path).unwrap();

    let (state_update_sender, state_update_recv) = watch::channel(
        bootstrap_state_update_info(&mut storage_manager)
            .await
            .unwrap(),
    );

    let (sync_sender, _sync_status_receiver) = watch::channel(SyncStatus::START);
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
    shutdown_receiver.mark_unchanged();

    let (prev_state_root, genesis_state_root) = init_variant
        .initialize(&stf, &mut storage_manager)
        .await
        .unwrap();

    let finalized_header = da_service.get_last_finalized_block_header().await.unwrap();
    let (_, ledger_state) = storage_manager
        .create_state_after(&finalized_header)
        .unwrap();
    let ledger_db = LedgerDb::with_reader(ledger_state).unwrap();

    let mut tasks = JoinSet::new();

    tasks.spawn({
        let ledger_updates = ledger_db.clone();
        react_to_state_updates::<TestSpec, _>(
            state_update_recv,
            shutdown_receiver.clone(),
            "ledger_updates",
            move |info| {
                let ledger_updates = ledger_updates.clone();
                async move {
                    ledger_updates.replace_reader(info.ledger_reader);
                    ledger_updates.send_notifications_for_slot(info.slot_number);
                    Ok(())
                }
            },
        )
    });

    let mut runner = StateTransitionRunner::new(
        rollup_config.runner.clone(),
        if nb_of_prover_threads.is_some() {
            Some(rollup_config.proof_manager)
        } else {
            None
        },
        da_service.clone(),
        ledger_db.clone(),
        stf,
        storage_manager,
        state_update_sender,
        prev_state_root,
        sync_sender,
        Box::new(InfiniteHeight),
        shutdown_receiver.clone(),
        rollup_config.monitoring.clone(),
        None,
        None,
    )
    .await
    .unwrap();

    if let Some(stf_info_receiver) = runner.take_stf_info_receiver() {
        let prover_service =
            ParallelProverService::<_, _, _, MockDaService, MockZkvm, MockZkvm>::new(
                inner_vm.clone(),
                outer_vm.clone(),
                verifier,
                RollupProverConfigDiscriminants::Prove,
                nb_of_prover_threads.unwrap(),
                Default::default(),
                MockAddress::new([0u8; 32]),
            );
        let handle = start_zk_workflow_in_background::<_>(
            prover_service,
            rollup_config.proof_manager.aggregated_proof_block_jump,
            Box::new(MockProofSender {
                da: da_service.clone(),
            }),
            genesis_state_root,
            stf_info_receiver,
            shutdown_receiver.clone(),
        )
        .await
        .unwrap();
        tasks.spawn(async move {
            let _ = handle.await;
        });
    }

    let proof_posted_in_da_sub = da_service.subscribe_proof_posted();
    let agg_proof_saved_in_db_sub: std::pin::Pin<
        Box<dyn Stream<Item = AggregatedProofResponse> + Send>,
    > = ledger_db.subscribe_proof_saved();

    (
        runner,
        TestNode {
            proof_posted_in_da_sub,
            agg_proof_saved_in_db_sub,
            da: da_service,
            inner_vm,
            _outer_vm: outer_vm,
            tasks,
            shutdown_sender,
            _sync_status_receiver,
        },
    )
}

type GenesisParams<ST, InnerVm, OuterVm, Da> =
    <ST as StateTransitionFunction<InnerVm, OuterVm, Da>>::GenesisParams;

/// How [`StateTransitionRunner`] is initialized
pub enum InitVariant<
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Da: DaService,
> {
    /// From give state root
    Initialized {
        prev_state_root: Stf::StateRoot,
        last_finalized_block_header: <Da::Spec as DaSpec>::BlockHeader,
    },
    /// From empty state root
    Genesis {
        /// Genesis block header should be finalized at an initialization moment.
        block: Da::FilteredBlock,
        /// Genesis params for Stf::init.
        genesis_params: GenesisParams<Stf, InnerVm, OuterVm, Da::Spec>,
    },
}

impl<Stf, InnerVm, OuterVm, Da> InitVariant<Stf, InnerVm, OuterVm, Da>
where
    Stf::PreState: NativeStorage<Root = Stf::StateRoot>,
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Da: DaService,
{
    pub async fn initialize<Sm>(
        self,
        stf: &Stf,
        storage_manager: &mut Sm,
    ) -> anyhow::Result<(Stf::StateRoot, Stf::StateRoot)>
    where
        Sm: HierarchicalStorageManager<
            Da::Spec,
            LedgerChangeSet = SchemaBatch,
            LedgerState = DeltaReader,
            StfState = Stf::PreState,
            StfChangeSet = Stf::ChangeSet,
        >,
    {
        let (prev_state_root, genesis_state_root) = match self {
            InitVariant::Initialized {
                prev_state_root,
                last_finalized_block_header,
            } => {
                let (prover_storage, _ledger_state) =
                    storage_manager.create_state_after(&last_finalized_block_header)?;
                let genesis_state_root = prover_storage.get_root_hash(SlotNumber::GENESIS)?;

                (prev_state_root, genesis_state_root)
            }
            InitVariant::Genesis {
                block,
                genesis_params: params,
            } => {
                let genesis_state_root = initialize_state::<Stf, InnerVm, OuterVm, Da, Sm>(
                    stf,
                    storage_manager,
                    block,
                    params,
                )
                .await?;
                (genesis_state_root.clone(), genesis_state_root)
            }
        };

        Ok((prev_state_root, genesis_state_root))
    }
}

fn get_da_polling_interval_ms(da_config: &MockDaConfig) -> u64 {
    match da_config.block_producing {
        BlockProducingConfig::Periodic { block_time_ms } =>
        // 10 times per block, but 10 ms in the worst case
        {
            block_time_ms.checked_div(5).unwrap_or(10)
        }
        _ => 150,
    }
}

pub fn rollup_config_with_da<Da: DaService<Config = MockDaConfig>>(
    path: &std::path::Path,
    da_config: MockDaConfig,
    aggregated_proof_block_jump: usize,
) -> RollupConfig<MockAddress, Da> {
    RollupConfig {
        storage: RollupDbConfig::default_in_path(path.to_path_buf()),
        runner: RunnerConfig {
            genesis_height: 0,
            da_polling_interval_ms: get_da_polling_interval_ms(&da_config),
            http_config: HttpServerConfig::localhost_on_free_port(),
            concurrent_sync_tasks: Some(1),
            save_tx_bodies: false,
        },
        da: da_config,
        proof_manager: ProofManagerConfig {
            aggregated_proof_block_jump: NonZero::new(aggregated_proof_block_jump).unwrap(),
            prover_address: MockAddress::new([0u8; 32]),
            max_number_of_transitions_in_db: NonZero::new(30).unwrap(),
            max_number_of_transitions_in_memory: NonZero::new(20).unwrap(),
        },
        sequencer: SequencerConfig {
            automatic_batch_production: true,
            max_allowed_node_distance_behind: 10,
            // Set ttl to zero to disable for testing. This prevents nondeterminism.
            dropped_tx_ttl_secs: 0,
            admin_addresses: vec![],
            rollup_address: MockAddress::new([0u8; 32]),
            sequencer_kind_config: SequencerKindConfig::Standard(StdSequencerConfig {
                mempool_max_txs_count: None,
                max_batch_size_bytes: None,
            }),
            max_batch_size_bytes: TEST_MAX_BATCH_SIZE,
            max_concurrent_blobs: TEST_MAX_CONCURRENT_BLOBS,
            blob_processing_timeout_secs: TEST_BLOB_PROCESSING_TIMEOUT,
        },
        monitoring: MonitoringConfig::standard(),
    }
}

fn rollup_config(
    da_service: &MockDaService,
    path: &std::path::Path,
    aggregated_proof_block_jump: usize,
) -> RollupConfig<MockAddress, MockDaService> {
    rollup_config_with_da::<MockDaService>(
        path,
        MockDaConfig::instant_with_sender(da_service.sequencer_address()),
        aggregated_proof_block_jump,
    )
}
