use std::sync::Arc;

use async_trait::async_trait;
use demo_stf::runtime::Runtime;
use demo_stf::MultiAddressEvmSolana;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::NomtStorageManager;
use sov_ethereum::EthRpcConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{
    MockCodeCommitment, MockZkVerifier, MockZkvm, MockZkvmCryptoSpec, MockZkvmHost,
};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::{Native, WitnessGeneration};
use sov_modules_api::{CryptoSpec, NodeEndpoints, Spec};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData,
};
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
use sov_stf_runner::processes::{ParallelProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

use crate::eth_dev_signer;
use crate::read_latest_aggregated_proof;
use crate::solana_offchain_endpoint::solana_offchain_router;

/// Rollup with a [`ConfigurableSpec`] with [`MockDaSpec`] as Da spec, [`MockZkvm`] inner vm and [`MockZkvm`] for outer vm
#[derive(Default, Clone, Copy)]
pub struct MockDemoRollup<M> {
    phantom: std::marker::PhantomData<M>,
}

type Hasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;
type NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Hasher>, <MockDaSpec as DaSpec>::SlotHash>;

/// The default spec of the rollup
pub type MockRollupSpec<M> = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    M,
    MockZkvmCryptoSpec,
    NativeStorage,
>;

impl RollupBlueprint<Native> for MockDemoRollup<Native>
where
    MockRollupSpec<Native>: PluggableSpec,
    <MockRollupSpec<Native> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = MockRollupSpec<Native>;
    type Runtime = Runtime<Self::Spec>;
}

impl RollupBlueprint<WitnessGeneration> for MockDemoRollup<WitnessGeneration>
where
    MockRollupSpec<WitnessGeneration>: PluggableSpec,
    <MockRollupSpec<WitnessGeneration> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = MockRollupSpec<WitnessGeneration>;
    type Runtime = Runtime<Self::Spec>;
}

#[async_trait]
impl FullNodeBlueprint<Native> for MockDemoRollup<Native> {
    type DaService = StorableMockDaService;

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
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> Self::DaService {
        StorableMockDaService::from_config(rollup_config.da.clone(), shutdown_receiver).await
    }

    async fn create_prover_service(
        &self,
        _prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        _da_service: &Self::DaService,
        ledger_db: &LedgerDb,
    ) -> (Self::ProverService, Option<SlotNumber>) {
        let previous_public_data: Option<
            AggregatedProofPublicData<
                <Self::Spec as Spec>::Address,
                MockDaSpec,
                <<Self::Spec as Spec>::Storage as Storage>::Root,
            >,
        > = read_latest_aggregated_proof(ledger_db).await.map(|proof| {
            AggregateProofVerifier::<MockZkVerifier>::new(MockCodeCommitment::default())
                .verify(&proof)
                .expect("Persisted aggregated proof failed verification")
        });

        let latest_proof_final_slot = previous_public_data.as_ref().map(|p| p.final_slot_number);

        let inner_vm = MockZkvmHost::new_non_blocking();
        let outer_vm =
            MockZkvmHost::new_non_blocking_with_previous_anchor(previous_public_data.as_ref());
        let da_verifier = Default::default();

        let num_threads = rollup_config
            .proof_manager
            .aggregated_proof_block_jump
            .get()
            + 1;

        let prover = ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            da_verifier,
            rollup_config.proof_manager.prover_address,
            num_threads,
        );

        (prover, latest_proof_final_slot)
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
        // MockZkvm has no ELF to derive a commitment from — the default zero
        // commitment matches what `MockZkvmHost::code_commitment` returns.
        Ok((MockCodeCommitment::default(), MockCodeCommitment::default()))
    }
}
