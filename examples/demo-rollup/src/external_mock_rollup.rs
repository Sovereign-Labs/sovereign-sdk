use std::sync::Arc;

use async_trait::async_trait;
use demo_stf::runtime::Runtime;
use demo_stf::MultiAddressEvmSolana;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::NomtStorageManager;
use sov_ethereum::EthRpcConfig;
use sov_mock_da::storable::rpc::StorableMockDaClient;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{MockCodeCommitment, MockZkvm, MockZkvmCryptoSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::{Native, WitnessGeneration};
use sov_modules_api::{NodeEndpoints, Spec, Storage};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::SyncStatus;
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_stf_runner::processes::{ParallelProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

use crate::eth_dev_signer;
use crate::solana_offchain_endpoint::solana_offchain_router;
use crate::{
    create_mock_prover_service, read_mock_code_commitments_from_env, Hasher, NativeStorage,
};

/// The default spec of the rollup
pub type ExternalMockRollupSpec<M> = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    M,
    MockZkvmCryptoSpec,
    NativeStorage,
>;

/// Rollup that connects to external mock-da.
#[derive(Default, Clone, Copy)]
pub struct ExternalMockDemoRollup<M> {
    phantom: std::marker::PhantomData<M>,
}

impl RollupBlueprint<Native> for ExternalMockDemoRollup<Native>
where
    ExternalMockRollupSpec<Native>: PluggableSpec,
    <ExternalMockRollupSpec<Native> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = ExternalMockRollupSpec<Native>;
    type Runtime = Runtime<Self::Spec>;
}

impl RollupBlueprint<WitnessGeneration> for ExternalMockDemoRollup<WitnessGeneration>
where
    ExternalMockRollupSpec<WitnessGeneration>: PluggableSpec,
    <ExternalMockRollupSpec<WitnessGeneration> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = ExternalMockRollupSpec<WitnessGeneration>;
    type Runtime = Runtime<Self::Spec>;
}

#[async_trait]
impl FullNodeBlueprint<Native> for ExternalMockDemoRollup<Native> {
    type DaService = StorableMockDaClient;

    type StorageManager = NomtStorageManager<MockDaSpec, Hasher, NativeStorage>;

    type ProverService = ParallelProverService<
        <Self::Spec as Spec>::Address,
        <<Self::Spec as Spec>::Storage as Storage>::Root,
        <<Self::Spec as Spec>::Storage as Storage>::Witness,
        Self::DaService,
        <Self::Spec as Spec>::InnerZkvm,
        <Self::Spec as Spec>::OuterZkvm,
    >;

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
        sov_modules_rollup_blueprint::register_endpoints::<Self, Native>(
            state_update_receiver.clone(),
            sync_status_receiver,
            shutdown_receiver,
            ledger_db,
            sequencer,
            rollup_config,
        )
        .await
    }

    async fn sequencer_additional_apis<Seq>(
        &self,
        sequencer: Seq,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        sequencer_da_address: <MockDaSpec as sov_modules_api::DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        let eth_signer = eth_dev_signer();
        let eth_rpc_config = EthRpcConfig {
            eth_signer,
            extension: rollup_config.extension_or_panic(),
            sequencer_rollup_address: rollup_config.sequencer.rollup_address,
            sequencer_da_address,
            sequencer_type: crate::sequencer_type(&rollup_config.sequencer),
            shutdown_receiver,
        };
        let axum_router = solana_offchain_router(sequencer.clone());

        Ok(NodeEndpoints {
            axum_router,
            jsonrpsee_module: sov_ethereum::get_ethereum_rpc(eth_rpc_config, sequencer)
                .remove_context(),
            ..Default::default()
        })
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _secondary_shutdown_controller: &sov_rollup_interface::node::SecondaryShutdownController,
    ) -> Self::DaService {
        StorableMockDaClient::from_config(rollup_config.da.clone())
            .expect("Failed to create da service")
    }

    async fn create_prover_service(
        &self,
        _prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _da_service: &Self::DaService,
        ledger_db: &LedgerDb,
        start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<(Self::ProverService, Option<SlotNumber>)> {
        create_mock_prover_service(rollup_config, ledger_db, start_fresh_outer_proof_on_resync)
            .await
    }

    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        witness_generation: bool,
    ) -> anyhow::Result<Self::StorageManager> {
        NomtStorageManager::new(rollup_config.storage.clone(), witness_generation)
    }

    fn create_proof_sender(
        &self,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        sequence_number_provider: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender> {
        Ok(Self::ProofSender::new(sequence_number_provider))
    }

    fn compute_code_commitments() -> anyhow::Result<(MockCodeCommitment, MockCodeCommitment)> {
        Ok(read_mock_code_commitments_from_env())
    }
}
