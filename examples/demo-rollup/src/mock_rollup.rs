use async_trait::async_trait;
use demo_stf::genesis_config::StorageConfig;
use demo_stf::runtime::Runtime;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::RollupTemplate;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::mocks::{MockDaConfig, MockDaService, MockDaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_state::state_manager::SovStateManager;
use sov_state::DefaultStorageSpec;
use sov_stf_runner::RollupConfig;

/// Rollup with MockDa
pub struct MockDemoRollup {}

#[async_trait]
impl RollupTemplate for MockDemoRollup {
    type DaService = MockDaService;
    type Vm = Risc0Host<'static>;
    type GenesisPaths = GenesisPaths<PathBuf>;

    type ZkContext = ZkDefaultContext;
    type NativeContext = DefaultContext;

    type StateManager = SovStateManager<DefaultStorageSpec>;

    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;
    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as Spec>::Storage,
        ledger_db: &LedgerDB,
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
        MockDaService::new(rollup_config.da.sender_address)
    }

    fn create_state_manager(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> anyhow::Result<Self::StateManager> {
        let storage_config = StorageConfig {
            path: rollup_config.storage.path.clone(),
        };
        SovStateManager::new(storage_config)
    }

    fn create_vm(&self) -> Self::Vm {
        Risc0Host::new(risc0::MOCK_DA_ELF)
    }

    fn create_verifier(&self) -> <Self::DaService as DaService>::Verifier {
        Default::default()
    }
}
