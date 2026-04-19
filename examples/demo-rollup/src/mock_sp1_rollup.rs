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
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::{Native, WitnessGeneration};
use sov_modules_api::{CryptoSpec, NodeEndpoints, Spec, ZkVerifier};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::zk::ZkvmHost;
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_sp1_adapter::host::{SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1CryptoSpec, SP1};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
use sov_stf_runner::processes::{
    ParallelProverService, ProverService, RollupProverConfig, RollupProverConfigDiscriminants,
};
use sov_stf_runner::RollupConfig;

use crate::eth_dev_signer;
use crate::solana_offchain_endpoint::solana_offchain_router;

/// Rollup with a [`ConfigurableSpec`] with [`MockDaSpec`] as Da spec, and [`SP1`] for both inner and outer vm
#[derive(Default, Clone, Copy)]
pub struct MockSp1DemoRollup<M> {
    phantom: std::marker::PhantomData<M>,
}

type Hasher = <SP1CryptoSpec as CryptoSpec>::Hasher;
type NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Hasher>, <MockDaSpec as DaSpec>::SlotHash>;

/// The default spec of the rollup
pub type MockSp1RollupSpec<M> = ConfigurableSpec<
    MockDaSpec,
    SP1,
    SP1,
    MultiAddressEvmSolana,
    M,
    SP1CryptoSpec,
    NativeStorage,
>;

impl RollupBlueprint<Native> for MockSp1DemoRollup<Native>
where
    MockSp1RollupSpec<Native>: PluggableSpec,
    <MockSp1RollupSpec<Native> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = MockSp1RollupSpec<Native>;
    type Runtime = Runtime<Self::Spec>;
}

impl RollupBlueprint<WitnessGeneration> for MockSp1DemoRollup<WitnessGeneration>
where
    MockSp1RollupSpec<WitnessGeneration>: PluggableSpec,
    <MockSp1RollupSpec<WitnessGeneration> as Spec>::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = MockSp1RollupSpec<WitnessGeneration>;
    type Runtime = Runtime<Self::Spec>;
}

#[async_trait]
impl FullNodeBlueprint<Native> for MockSp1DemoRollup<Native> {
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

    fn create_outer_code_commitment(
        &self,
    ) -> <<Self::ProverService as ProverService>::Verifier as ZkVerifier>::CodeCommitment {
        let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;
        SP1Host::new(agg_elf)
            .expect("Failed to create SP1Host from aggregation guest ELF")
            .code_commitment()
            .expect("SP1 aggregation code commitment should be created successfully")
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
    ) -> Self::ProverService {
        let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;

        // SP1's blocking CPU prover spins up its own tokio runtime during setup,
        // so it must be constructed off the async executor thread.
        let inner_vm = tokio::task::spawn_blocking(move || SP1Host::new(elf))
            .await
            .expect("SP1Host setup task panicked")
            .expect("Failed to create SP1Host from guest ELF");

        let inner_vm_clone = inner_vm.clone();
        let inner_code_commitment = tokio::task::spawn_blocking(move || {
            inner_vm_clone
                .code_commitment()
                .expect("SP1 inner code commitment should be created successfully")
        })
        .await
        .expect("SP1 inner code commitment task panicked");

        let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;
        let outer_vm = tokio::task::spawn_blocking(move || {
            SP1AggregationHost::new(agg_elf, inner_code_commitment)
                .expect("Failed to create SP1AggregationHost from aggregation guest ELF")
        })
        .await
        .expect("SP1AggregationHost setup task panicked");

        let da_verifier = Default::default();

        ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            da_verifier,
            RollupProverConfigDiscriminants::Prove,
            rollup_config.proof_manager.prover_address,
        )
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
