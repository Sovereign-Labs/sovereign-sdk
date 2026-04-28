use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use rockbound::SchemaBatch;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::DeltaReader;
use sov_db::storage_manager::{NativeStorageManager, NomtStorageManager};
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{MockDaSpec, MockHash};
use sov_mock_zkvm::{MockZkvm, MockZkvmHost};
use sov_modules_api::capabilities::{HasCapabilities, HasKernel};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::rest::HasRestApi;
use sov_modules_api::{CodeCommitmentFor, CryptoSpec, NodeEndpoints, Spec, Zkvm};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::ZkvmGuest;
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, ProverStorage, Storage};
use sov_stf_runner::processes::{ParallelProverService, ProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

/// Factory for creating a prover service within [`RtAgnosticBlueprint`].
///
/// Implement this trait to plug different prover backends (parallel, network, etc.)
/// into the blueprint without reimplementing the entire [`FullNodeBlueprint`].
#[async_trait]
pub trait ProverFactory<S: Spec<Da = MockDaSpec>>: Send + Sync + 'static {
    /// The prover service type this factory creates.
    type ProverService: ProverService<
        StateRoot = <S::Storage as Storage>::Root,
        Witness = <S::Storage as Storage>::Witness,
        DaService = StorableMockDaService,
        Verifier = <<S::OuterZkvm as Zkvm>::Guest as ZkvmGuest>::Verifier,
    >;

    /// Create the prover service from the given config.
    async fn create(
        prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<S::Address, StorableMockDaService>,
    ) -> Self::ProverService;

    /// Compute the inner+outer code commitments for this prover, typically by
    /// hashing the guest ELFs. Default impl bails — factories that want to
    /// support `--override-code-commitments`-style genesis rewrites should
    /// override this.
    #[allow(clippy::type_complexity)]
    fn code_commitments() -> anyhow::Result<(
        CodeCommitmentFor<S::InnerZkvm>,
        CodeCommitmentFor<S::OuterZkvm>,
    )> {
        anyhow::bail!("code_commitments not supported by this prover factory")
    }
}

/// Default prover factory using local parallel proving.
pub struct ParallelProverFactory<S>(PhantomData<S>);

#[async_trait]
impl<S> ProverFactory<S> for ParallelProverFactory<S>
where
    S: Spec<Da = MockDaSpec, InnerZkvm = MockZkvm, OuterZkvm = MockZkvm> + PluggableSpec,
{
    type ProverService = ParallelProverService<
        S::Address,
        <S::Storage as Storage>::Root,
        <S::Storage as Storage>::Witness,
        StorableMockDaService,
        S::InnerZkvm,
        S::OuterZkvm,
    >;

    async fn create(
        _prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<S::Address, StorableMockDaService>,
    ) -> Self::ProverService {
        let inner_vm = MockZkvmHost::new_non_blocking();
        let outer_vm = MockZkvmHost::new_non_blocking();

        ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            Default::default(),
            rollup_config.proof_manager.prover_address,
            5,
        )
    }
}

/// A basic, "vanilla" [`FullNodeBlueprint`] to be used for testing.
///
/// The `A` parameter allows injecting additional sequencer APIs (e.g., EVM's `eth_*`
/// methods). It defaults to [`NoAdditionalApis`], which returns empty endpoints.
pub struct RtAgnosticBlueprint<
    S: Spec,
    R: RuntimeTrait<S>,
    Manager = NomtStorageManager<
        MockDaSpec,
        <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher,
        NomtProverStorage<
            DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
            MockHash,
        >,
    >,
    Prover = ParallelProverFactory<S>,
    A = NoAdditionalApis,
> {
    phantom: PhantomData<(S, R, Manager, Prover, A)>,
}

/// [`RtAgnosticBlueprint`] with the default NOMT storage manager and custom additional APIs.
pub type RtAgnosticBlueprintWithApis<S, R, A> = RtAgnosticBlueprint<
    S,
    R,
    NomtStorageManager<
        MockDaSpec,
        <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher,
        NomtProverStorage<
            DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
            MockHash,
        >,
    >,
    ParallelProverFactory<S>,
    A,
>;

impl<S: Spec, R: RuntimeTrait<S>, Manager, Prover, A> Default
    for RtAgnosticBlueprint<S, R, Manager, Prover, A>
{
    fn default() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<S, R, Manager, Prover, A> RollupBlueprint<Native>
    for RtAgnosticBlueprint<S, R, Manager, Prover, A>
where
    S: Spec + PluggableSpec,
    R: RuntimeTrait<S> + HasKernel<S> + HasCapabilities<S> + HasKernel<S>,
    Manager: Send + Sync + 'static,
    Prover: Send + Sync + 'static,
    A: Send + Sync + 'static,
{
    type Spec = S;
    type Runtime = R;
}

#[async_trait]
impl<S, R, Manager, Prover, A> FullNodeBlueprint<Native>
    for RtAgnosticBlueprint<S, R, Manager, Prover, A>
where
    S: Spec<Da = MockDaSpec> + PluggableSpec,
    R: RuntimeTrait<S> + HasRestApi<S> + HasCapabilities<S> + HasKernel<S> + 'static,
    Manager: Send
        + Sync
        + 'static
        + HierarchicalStorageManager<
            MockDaSpec,
            StfState = <S as Spec>::Storage,
            LedgerChangeSet = SchemaBatch,
            LedgerState = DeltaReader,
            StfChangeSet = <S::Storage as Storage>::ChangeSet,
        >
        + StorageManagerInitializer<S, StorableMockDaService>,
    Prover: ProverFactory<S>,
    A: AdditionalSequencerApis<S, R>,
{
    type DaService = StorableMockDaService;

    type StorageManager = Manager;

    type ProverService = <Prover as ProverFactory<S>>::ProverService;

    type ProofSender = SovApiProofSender<Self::Spec>;

    async fn create_endpoints(
        &self,
        state_update_receiver: StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        sync_status_receiver: tokio::sync::watch::Receiver<SyncStatus>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        ledger_db: &LedgerDb,
        sequencer: &SequencerCreationReceipt<Self::Spec>,
        _da_service: &Self::DaService,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints> {
        Ok(
            sov_modules_rollup_blueprint::register_endpoints::<Self, Native>(
                state_update_receiver,
                sync_status_receiver,
                shutdown_receiver,
                ledger_db,
                sequencer,
                rollup_config,
            )
            .await?,
        )
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> Self::DaService {
        StorableMockDaService::from_config(rollup_config.da.clone(), shutdown_receiver).await
    }

    async fn create_prover_service(
        &self,
        prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _da_service: &Self::DaService,
        _ledger_db: &LedgerDb,
    ) -> (
        Self::ProverService,
        Option<sov_rollup_interface::common::SlotNumber>,
    ) {
        (Prover::create(prover_config, rollup_config).await, None)
    }

    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        witness_generation: bool,
    ) -> anyhow::Result<Self::StorageManager> {
        Manager::from_config(rollup_config, witness_generation)
    }

    fn create_proof_sender(
        &self,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        proof_blob_sender: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender> {
        Ok(Self::ProofSender::new(proof_blob_sender))
    }

    #[allow(clippy::type_complexity)]
    fn compute_code_commitments() -> anyhow::Result<(
        CodeCommitmentFor<<S as Spec>::InnerZkvm>,
        CodeCommitmentFor<<S as Spec>::OuterZkvm>,
    )> {
        Prover::code_commitments()
    }

    async fn sequencer_additional_apis<Seq>(
        &self,
        sequencer: Seq,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        sequencer_da_address: <<Self::Spec as Spec>::Da as sov_rollup_interface::da::DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        A::create(
            sequencer,
            rollup_config,
            shutdown_receiver,
            sequencer_da_address,
        )
    }
}

trait StorageManagerInitializer<S: Spec, Da: DaService>: Sized {
    fn from_config(
        config: &RollupConfig<S::Address, Da>,
        witness_generation: bool,
    ) -> anyhow::Result<Self>;
}

impl<S: Spec> StorageManagerInitializer<S, StorableMockDaService>
    for NativeStorageManager<
        MockDaSpec,
        ProverStorage<DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>>,
    >
{
    fn from_config(
        config: &RollupConfig<<S as Spec>::Address, StorableMockDaService>,
        _witness_generation: bool,
    ) -> anyhow::Result<Self> {
        NativeStorageManager::new(&config.storage.path)
    }
}

impl<S: Spec> StorageManagerInitializer<S, StorableMockDaService>
    for NomtStorageManager<
        MockDaSpec,
        <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher,
        NomtProverStorage<
            DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
            MockHash,
        >,
    >
{
    fn from_config(
        config: &RollupConfig<<S as Spec>::Address, StorableMockDaService>,
        witness_generation: bool,
    ) -> anyhow::Result<Self> {
        NomtStorageManager::new(config.storage.clone(), witness_generation)
    }
}

/// Trait for injecting additional sequencer HTTP/RPC endpoints into
/// [`RtAgnosticBlueprint`]. Implement this and pass as the 4th generic parameter `A`.
pub trait AdditionalSequencerApis<S: Spec<Da = MockDaSpec>, R: RuntimeTrait<S>>:
    Default + Send + Sync + 'static
{
    /// Creates additional [`NodeEndpoints`] for the sequencer.
    ///
    /// The returned endpoints will be merged into the sequencer's API surface.
    fn create<Seq>(
        sequencer: Seq,
        rollup_config: &RollupConfig<S::Address, StorableMockDaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        sequencer_da_address: <MockDaSpec as sov_rollup_interface::da::DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = S, Rt = R, Da = StorableMockDaService>;
}

/// Default implementation of [`AdditionalSequencerApis`] that returns empty endpoints.
#[derive(Default)]
pub struct NoAdditionalApis;

impl<S, R> AdditionalSequencerApis<S, R> for NoAdditionalApis
where
    S: Spec<Da = MockDaSpec>,
    R: RuntimeTrait<S>,
{
    fn create<Seq>(
        _sequencer: Seq,
        _rollup_config: &RollupConfig<S::Address, StorableMockDaService>,
        _shutdown_receiver: tokio::sync::watch::Receiver<()>,
        _sequencer_da_address: <MockDaSpec as sov_rollup_interface::da::DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = S, Rt = R, Da = StorableMockDaService>,
    {
        Ok(NodeEndpoints::default())
    }
}
