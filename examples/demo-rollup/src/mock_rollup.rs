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
use sov_state::{ProverStorage, Storage, ZkStorage};
use sov_stf_runner::RollupConfig;

/// Rollup with MockDa
pub struct MockDemoRollup {}

#[async_trait]
impl RollupTemplate for MockDemoRollup {
    type DaService = MockDaService;
    type Vm = Risc0Host<'static>;

    type ZkContext = ZkDefaultContext;
    type NativeContext = DefaultContext;

    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;
    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    type DaSpec = MockDaSpec;
    type DaConfig = MockDaConfig;

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Self::DaService {
        MockDaService::new(rollup_config.da.sender_address)
    }

    fn create_vm(&self) -> Self::Vm {
        Risc0Host::new(risc0::MOCK_DA_ELF)
    }

    fn create_zk_storage(
        &self,
        _rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::ZkContext as Spec>::Storage {
        ZkStorage::new()
    }

    fn create_verifier(&self) -> <Self::DaService as DaService>::Verifier {
        Default::default()
    }

    fn create_native_storage(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Result<<Self::NativeContext as sov_modules_api::Spec>::Storage, anyhow::Error> {
        let storage_config = StorageConfig {
            path: rollup_config.storage.path.clone(),
        };
        ProverStorage::with_config(storage_config)
    }

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
}
