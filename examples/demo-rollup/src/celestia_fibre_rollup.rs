use std::sync::Arc;

use async_trait::async_trait;
use demo_stf::runtime::Runtime;
use demo_stf::MultiAddressEvmSolana;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_fibre_adapter::{CelestiaFibreService, CelestiaFibreSpec, CelestiaFibreVerifier};
use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::NomtStorageManager;
use sov_ethereum::EthRpcConfig;
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec, MockZkvmHost};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{NodeEndpoints, Spec, Storage};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{
    FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt, WalletBlueprint,
};
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::execution_mode::WitnessGeneration;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_stf_runner::processes::{ParallelProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

use crate::solana_offchain_endpoint::solana_offchain_router;
use crate::{eth_dev_signer, ROLLUP_BATCH_NAMESPACE, ROLLUP_PROOF_NAMESPACE};

/// Rollup on Celestia Fibre, with [`MockZkvm`] as both the inner and outer VM.
#[derive(Default, Clone, Copy)]
pub struct CelestiaFibreDemoRollup<M> {
    phantom: std::marker::PhantomData<M>,
}

type Hasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;
type NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Hasher>, <CelestiaFibreSpec as DaSpec>::SlotHash>;

type CelestiaFibreRollupSpec<M> = ConfigurableSpec<
    CelestiaFibreSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    M,
    MockZkvmCryptoSpec,
    NativeStorage,
>;

fn rollup_params() -> RollupParams {
    RollupParams {
        rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
        rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
    }
}

impl RollupBlueprint<Native> for CelestiaFibreDemoRollup<Native>
where
    CelestiaFibreRollupSpec<Native>: PluggableSpec,
    <CelestiaFibreRollupSpec<Native> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = CelestiaFibreRollupSpec<Native>;
    type Runtime = Runtime<Self::Spec>;
}

impl RollupBlueprint<WitnessGeneration> for CelestiaFibreDemoRollup<WitnessGeneration>
where
    CelestiaFibreRollupSpec<WitnessGeneration>: PluggableSpec,
    <CelestiaFibreRollupSpec<WitnessGeneration> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = CelestiaFibreRollupSpec<WitnessGeneration>;
    type Runtime = Runtime<Self::Spec>;
}

#[async_trait]
impl FullNodeBlueprint<Native> for CelestiaFibreDemoRollup<Native> {
    type DaService = CelestiaFibreService;
    type StorageManager = NomtStorageManager<CelestiaFibreSpec, Hasher, NativeStorage>;
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
        primary_shutdown: sov_shutdown::PrimaryShutdownController,
        ledger_db: &LedgerDb,
        sequencer: &SequencerCreationReceipt<Self::Spec>,
        _da_service: &Self::DaService,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints> {
        sov_modules_rollup_blueprint::register_endpoints::<Self, _>(
            state_update_receiver.clone(),
            sync_status_receiver,
            primary_shutdown,
            ledger_db,
            sequencer,
            rollup_config,
        )
        .await
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        secondary_shutdown_controller: &sov_shutdown::SecondaryShutdownController,
    ) -> Self::DaService {
        CelestiaFibreService::new(
            rollup_config.da.clone(),
            rollup_params(),
            secondary_shutdown_controller,
        )
        .await
        .expect("Failed to create Celestia Fibre DA service")
    }

    async fn sequencer_additional_apis<Seq>(
        &self,
        sequencer: Seq,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        primary_shutdown: sov_shutdown::PrimaryShutdownController,
        sequencer_da_address: <CelestiaFibreSpec as DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        let eth_rpc_config = EthRpcConfig {
            eth_signer: eth_dev_signer(),
            extension: rollup_config.extension_or_panic(),
            sequencer_rollup_address: rollup_config.sequencer.rollup_address,
            sequencer_da_address,
            sequencer_type: crate::sequencer_type(&rollup_config.sequencer),
            primary_shutdown,
        };
        let axum_router = solana_offchain_router(sequencer.clone());
        Ok(NodeEndpoints {
            axum_router,
            jsonrpsee_module: sov_ethereum::get_ethereum_rpc(eth_rpc_config, sequencer)
                .remove_context(),
        })
    }

    async fn create_prover_service(
        &self,
        _prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _da_service: &Self::DaService,
        _ledger_db: &LedgerDb,
        _start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<(
        Self::ProverService,
        Option<sov_rollup_interface::common::SlotNumber>,
    )> {
        let proof_manager = rollup_config
            .proof_manager
            .as_ref()
            .expect("proof_manager must be set when prover is enabled");
        let prover = ParallelProverService::new_with_default_workers(
            MockZkvmHost::new_non_blocking(),
            MockZkvmHost::new_non_blocking(),
            CelestiaFibreVerifier::new(rollup_params()),
            proof_manager.prover_address,
            proof_manager.prover_thread_count(),
        );
        Ok((prover, None))
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
}

impl WalletBlueprint<Native> for CelestiaFibreDemoRollup<Native> {}
