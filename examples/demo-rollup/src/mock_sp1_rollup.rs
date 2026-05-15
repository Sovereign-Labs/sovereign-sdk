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
use sov_modules_api::{CodeCommitmentTrait, CryptoSpec, NodeEndpoints, Spec};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::proof_sender::SovApiProofSender;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_rollup_full_node_interface::StateUpdateReceiver;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::SyncStatus;
use sov_sequencer::{ProofBlobSender, Sequencer};
use sov_sp1_adapter::host::{code_commitment_from_verifying_key, SP1AggregationHost, SP1Host};
use sov_sp1_adapter::{SP1CryptoSpec, SP1MethodId, SP1Verifier, SP1};
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{DefaultStorageSpec, Storage};
use sov_stf_runner::processes::{ParallelProverService, RollupProverConfig};
use sov_stf_runner::RollupConfig;

use crate::eth_dev_signer;
use crate::get_persisted_outer_proof;
use crate::solana_offchain_endpoint::solana_offchain_router;

/// Output of [`build_sp1_inner_host`]: the inner [`SP1Host`] together with the
/// inner and outer code commitments derived from the same setup pass.
struct Sp1InnerHostBundle {
    inner_host: SP1Host,
    inner_code_commitment: SP1MethodId,
    outer_code_commitment: SP1MethodId,
}

/// Runs the SP1 prover setup once per ELF and returns the inner host plus both
/// code commitments. Callers that only need the commitments can drop
/// `inner_host`; the setup cost is identical either way.
fn build_sp1_inner_host(inner_elf: &[u8], outer_elf: &[u8]) -> anyhow::Result<Sp1InnerHostBundle> {
    let outer_vk = Arc::new(sov_sp1_adapter::host::verifying_key_from_elf(outer_elf)?);
    let outer_code_commitment = code_commitment_from_verifying_key(&outer_vk);
    let inner_host = SP1Host::new(inner_elf, outer_vk)?;
    let inner_code_commitment = code_commitment_from_verifying_key(inner_host.verifying_key());
    Ok(Sp1InnerHostBundle {
        inner_host,
        inner_code_commitment,
        outer_code_commitment,
    })
}

/// Rollup with a [`ConfigurableSpec`] with [`MockDaSpec`] as Da spec, and [`SP1`] for both inner and outer vm
#[derive(Default, Clone, Copy)]
pub struct MockSp1DemoRollup<M> {
    phantom: std::marker::PhantomData<M>,
}

type Hasher = <SP1CryptoSpec as CryptoSpec>::Hasher;
type NativeStorage =
    NomtProverStorage<DefaultStorageSpec<Hasher>, <MockDaSpec as DaSpec>::SlotHash>;

/// The default spec of the rollup
pub type MockSp1RollupSpec<M> =
    ConfigurableSpec<MockDaSpec, SP1, SP1, MultiAddressEvmSolana, M, SP1CryptoSpec, NativeStorage>;

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
        start_fresh_outer_proof_on_resync: bool,
    ) -> anyhow::Result<(Self::ProverService, Option<SlotNumber>)> {
        let elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        let agg_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

        // SP1's blocking CPU prover spins up its own tokio runtime during setup,
        // so it must be constructed off the async executor thread.
        let Sp1InnerHostBundle {
            inner_host: inner_vm,
            inner_code_commitment,
            outer_code_commitment,
        } = tokio::task::spawn_blocking(move || build_sp1_inner_host(elf, agg_elf))
            .await
            .expect("SP1 host setup task panicked")?;

        let inner_verifying_key = inner_vm.verifying_key().clone();

        let (previous_for_outer, latest_proof_final_slot) = get_persisted_outer_proof::<
            SP1Verifier,
            <Self::Spec as Spec>::Address,
            MockDaSpec,
            <<Self::Spec as Spec>::Storage as Storage>::Root,
        >(
            ledger_db,
            inner_code_commitment.to_hash(),
            outer_code_commitment.to_hash(),
            start_fresh_outer_proof_on_resync,
        )
        .await?;

        let outer_vm = tokio::task::spawn_blocking(move || {
            SP1AggregationHost::new_with_previous_proof(
                agg_elf,
                inner_verifying_key,
                previous_for_outer,
            )
            .expect("Failed to create SP1AggregationHost from aggregation guest ELF")
        })
        .await
        .expect("SP1AggregationHost setup task panicked");

        let da_verifier = Default::default();

        let num_threads = rollup_config.proof_manager.prover_thread_count();
        let prover = ParallelProverService::new_with_default_workers(
            inner_vm,
            outer_vm,
            da_verifier,
            rollup_config.proof_manager.prover_address,
            num_threads,
        );

        Ok((prover, latest_proof_final_slot))
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

    fn compute_code_commitments() -> anyhow::Result<(SP1MethodId, SP1MethodId)> {
        let inner_elf: &[u8] = *sp1::SP1_GUEST_MOCK_ELF;
        let outer_elf: &[u8] = *sp1::SP1_GUEST_AGGREGATION_MOCK_ELF;

        let Sp1InnerHostBundle {
            inner_code_commitment,
            outer_code_commitment,
            ..
        } = build_sp1_inner_host(inner_elf, outer_elf)?;

        Ok((inner_code_commitment, outer_code_commitment))
    }
}
