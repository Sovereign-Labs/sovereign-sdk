use std::path::PathBuf;

use async_trait::async_trait;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths, StorageConfig};
use demo_stf::runtime::Runtime;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::RollupTemplate;
use sov_modules_stf_template::Runtime as RuntimeTrait;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::mocks::{MockDaConfig, MockDaService, MockDaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_state::{ProverStorage, Storage, ZkStorage};
use sov_stf_runner::RollupConfig;

use crate::common::create_rpc_methods;
#[cfg(feature = "experimental")]
use crate::common::read_eth_tx_signers;

/// Rollup with MockDa
pub struct MockDemoRollup {}

#[async_trait]
impl RollupTemplate for MockDemoRollup {
    type DaService = MockDaService;
    type GenesisPaths = GenesisPaths<PathBuf>;
    type Vm = Risc0Host<'static>;

    type ZkContext = ZkDefaultContext;
    type NativeContext = DefaultContext;

    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;
    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    type DaSpec = MockDaSpec;
    type DaConfig = MockDaConfig;

    fn create_genesis_config(
        &self,
        genesis_paths: &Self::GenesisPaths,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisConfig
    {
        #[cfg(feature = "experimental")]
        let eth_signer = read_eth_tx_signers();

        get_genesis_config(
            rollup_config.da.sender_address,
            genesis_paths,
            #[cfg(feature = "experimental")]
            eth_signer.signers(),
        )
    }

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
        create_rpc_methods(storage, ledger_db, da_service.clone())
    }
}
