use async_trait::async_trait;
use demo_stf::genesis_config::StorageConfig;
use demo_stf::runtime::Runtime;
use sov_celestia_adapter::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use sov_celestia_adapter::{CelestiaConfig, CelestiaService};
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::{RollupBlueprint, WalletBlueprint};
use sov_modules_stf_template::kernels::basic::BasicKernel;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::services::da::DaService;
use sov_state::storage_manager::ProverStorageManager;
use sov_state::{DefaultStorageSpec, ZkStorage};
use sov_stf_runner::RollupConfig;

use crate::ROLLUP_NAMESPACE;

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

    type StorageManager = ProverStorageManager<DefaultStorageSpec>;
    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;

    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    type NativeKernel = BasicKernel<Self::NativeContext>;
    type ZkKernel = BasicKernel<Self::ZkContext>;

    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as sov_modules_api::Spec>::Storage,
        ledger_db: &sov_db::ledger_db::LedgerDB,
        da_service: &Self::DaService,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
        #[allow(unused_mut)]
        let mut rpc_methods = sov_modules_rollup_template::register_rpc::<
            Self::NativeRuntime,
            Self::NativeContext,
            Self::DaService,
        >(storage, ledger_db, da_service)?;

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
                namespace: ROLLUP_NAMESPACE,
            },
        )
        .await
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

    fn create_zk_storage(
        &self,
        _rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::ZkContext as Spec>::Storage {
        ZkStorage::new()
    }

    fn create_vm(&self) -> Self::Vm {
        Risc0Host::new(risc0::ROLLUP_ELF)
    }

    fn create_verifier(&self) -> <Self::DaService as DaService>::Verifier {
        CelestiaVerifier {
            rollup_namespace: ROLLUP_NAMESPACE,
        }
    }
}

impl WalletBlueprint for CelestiaDemoRollup {}
