use std::marker::PhantomData;
use std::sync::Arc;

use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::NativeStorageManager;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{MockCodeCommitment, MockZkvm, MockZkvmHost};
use sov_modules_api::capabilities::{HasCapabilities, HasKernel};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::rest::{HasRestApi, StateUpdateReceiver};
use sov_modules_api::{CryptoSpec, NodeEndpoints, Spec, SyncStatus, ZkVerifier, Zkvm};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_rollup_interface::zk::aggregated_proof::CodeCommitment;
use sov_rollup_interface::zk::ZkvmHost;
use sov_sequencer::ProofBlobSender;
use sov_state::{DefaultStorageSpec, ProverStorage, Storage};
use sov_stf_runner::processes::{ParallelProverService, ProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

/// A basic, "vanilla" [`FullNodeBlueprint`] to be used for testing.
#[derive(Default)]
pub struct RtAgnosticBlueprint<S: Spec, R: RuntimeTrait<S>> {
    phantom: PhantomData<(S, R)>,
}

impl<S, R> RollupBlueprint<Native> for RtAgnosticBlueprint<S, R>
where
    S: Spec + PluggableSpec,
    R: RuntimeTrait<S> + HasKernel<S> + HasCapabilities<S> + HasKernel<S>,
{
    type Spec = S;
    type Runtime = R;
}

#[async_trait]
impl<S, R> FullNodeBlueprint<Native> for RtAgnosticBlueprint<S, R>
where
    S: Spec<
            Da = MockDaSpec,
            OuterZkvm = MockZkvm,
            Storage = ProverStorage<
                DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
            >,
        > + PluggableSpec,
    R: RuntimeTrait<S> + HasRestApi<S> + HasCapabilities<S> + HasKernel<S> + 'static,
{
    type DaService = StorableMockDaService;

    type StorageManager = NativeStorageManager<
        MockDaSpec,
        ProverStorage<DefaultStorageSpec<<<Self::Spec as Spec>::CryptoSpec as CryptoSpec>::Hasher>>,
    >;

    type ProverService = ParallelProverService<
        <Self::Spec as Spec>::Address,
        <<Self::Spec as Spec>::Storage as Storage>::Root,
        <<Self::Spec as Spec>::Storage as Storage>::Witness,
        Self::DaService,
        <Self::Spec as Spec>::InnerZkvm,
        <Self::Spec as Spec>::OuterZkvm,
    >;

    type ProofSender = SovApiProofSender<Self::Spec>;

    fn create_outer_code_commitment(
        &self,
    ) -> <<Self::ProverService as ProverService>::Verifier as ZkVerifier>::CodeCommitment {
        MockCodeCommitment::default()
    }

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
        prover_config: RollupProverConfig<<Self::Spec as Spec>::InnerZkvm>,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _da_service: &Self::DaService,
    ) -> Self::ProverService {
        let (host_args, prover_config_disc) = prover_config.split();

        let inner_vm = <S::InnerZkvm as Zkvm>::Host::from_args(&host_args);
        let outer_vm = MockZkvmHost::new_non_blocking();

        let da_verifier = Default::default();

        ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            da_verifier,
            prover_config_disc,
            CodeCommitment::default(),
            rollup_config.proof_manager.prover_address.clone(),
        )
    }

    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<Self::StorageManager> {
        NativeStorageManager::new(&rollup_config.storage.path)
    }

    fn create_proof_sender(
        &self,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        proof_blob_sender: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender> {
        Ok(Self::ProofSender::new(proof_blob_sender))
    }
}
