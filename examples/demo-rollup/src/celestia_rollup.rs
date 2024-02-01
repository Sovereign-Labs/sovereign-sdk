use async_trait::async_trait;
use demo_stf::genesis_config::StorageConfig;
use demo_stf::runtime::Runtime;
use sov_celestia_adapter::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use sov_celestia_adapter::{CelestiaConfig, CelestiaService};
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::{Address, Spec};
use sov_modules_rollup_blueprint::{RollupBlueprint, WalletBlueprint};
use sov_modules_stf_blueprint::kernels::basic::BasicKernel;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_prover_storage_manager::ProverStorageManager;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::{DefaultStorageSpec, Storage, ZkStorage};
use sov_stf_runner::{ParallelProverService, RollupConfig, RollupProverConfig};

use crate::{ROLLUP_BATCH_NAMESPACE, ROLLUP_PROOF_NAMESPACE};

/// Rollup with CelestiaDa
pub struct CelestiaDemoRollup {}

#[async_trait]
impl RollupBlueprint for CelestiaDemoRollup {
    type DaService = CelestiaService;
    type DaSpec = CelestiaSpec;
    type DaConfig = CelestiaConfig;
    type Vm = Risc0Host<'static>;

    type ZkContext = ZkDefaultContext;
    type NativeContext = DefaultContext;

    type StorageManager = ProverStorageManager<CelestiaSpec, DefaultStorageSpec>;
    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;

    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    type NativeKernel = BasicKernel<Self::NativeContext, Self::DaSpec>;
    type ZkKernel = BasicKernel<Self::ZkContext, Self::DaSpec>;

    type ProverService = ParallelProverService<
        <<Self::NativeContext as Spec>::Storage as Storage>::Root,
        <<Self::NativeContext as Spec>::Storage as Storage>::Witness,
        Self::DaService,
        Self::Vm,
        StfBlueprint<
            Self::ZkContext,
            Self::DaSpec,
            <Self::Vm as ZkvmHost>::Guest,
            Self::ZkRuntime,
            Self::ZkKernel,
        >,
    >;

    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as sov_modules_api::Spec>::Storage,
        ledger_db: &sov_db::ledger_db::LedgerDB,
        da_service: &Self::DaService,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
        // TODO set the sequencer address
        let sequencer = Address::new([0; 32]);

        #[allow(unused_mut)]
        let mut rpc_methods = sov_modules_rollup_blueprint::register_rpc::<
            Self::NativeRuntime,
            Self::NativeContext,
            Self::DaService,
        >(storage, ledger_db, da_service, sequencer)?;

        #[cfg(feature = "experimental")]
        crate::eth::register_ethereum::<Self::DaService>(
            da_service.clone(),
            storage.clone(),
            &mut rpc_methods,
        )?;

        Ok(rpc_methods)
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Self::DaService {
        CelestiaService::new(
            rollup_config.da.clone(),
            RollupParams {
                rollup_batch_namespace: ROLLUP_BATCH_NAMESPACE,
                rollup_proof_namespace: ROLLUP_PROOF_NAMESPACE,
            },
        )
        .await
    }

    async fn create_prover_service(
        &self,
        prover_config: RollupProverConfig,
        rollup_config: &RollupConfig<Self::DaConfig>,
        _da_service: &Self::DaService,
    ) -> Self::ProverService {
        let vm = Risc0Host::new(risc0::ROLLUP_ELF);
        let zk_stf = StfBlueprint::new();
        let zk_storage = ZkStorage::new();

        let da_verifier = CelestiaVerifier {
            rollup_namespace: ROLLUP_BATCH_NAMESPACE,
        };

        ParallelProverService::new_with_default_workers(
            vm,
            zk_stf,
            da_verifier,
            prover_config,
            zk_storage,
            rollup_config.prover_service,
        )
    }

    fn create_storage_manager(
        &self,
        rollup_config: &sov_stf_runner::RollupConfig<Self::DaConfig>,
    ) -> Result<Self::StorageManager, anyhow::Error> {
        let storage_config = StorageConfig {
            path: rollup_config.storage.path.clone(),
        };
        ProverStorageManager::new(storage_config)
    }
}

impl WalletBlueprint for CelestiaDemoRollup {}
