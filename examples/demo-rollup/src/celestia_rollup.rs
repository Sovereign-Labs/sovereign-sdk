use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths, StorageConfig};
use demo_stf::runtime::Runtime;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::{CelestiaSpec, CelestiaVerifier, RollupParams};
use sov_celestia_adapter::{CelestiaService, DaServiceConfig};
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::RollupTemplate;
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::services::da::DaService;
use sov_state::{ProverStorage, Storage, ZkStorage};
use sov_stf_runner::RollupConfig;

use crate::common::create_rpc_methods;
#[cfg(feature = "experimental")]
use crate::common::read_eth_tx_signers;
use crate::ROLLUP_NAMESPACE;

/// Rollup with CelestiaDa
pub struct CelestiaDemoRollup {}

#[async_trait]
impl RollupTemplate for CelestiaDemoRollup {
    type DaService = CelestiaService;
    type GenesisPaths = GenesisPaths<PathBuf>;
    type Vm = Risc0Host<'static>;

    type ZkContext = ZkDefaultContext;
    type NativeContext = DefaultContext;

    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;
    type NativeRuntime = Runtime<Self::NativeContext, Self::DaSpec>;

    type DaSpec = CelestiaSpec;
    type DaConfig = DaServiceConfig;

    fn create_genesis_config(
        &self,
        genesis_paths: &Self::GenesisPaths,
        _rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::NativeRuntime as sov_modules_stf_template::Runtime<
        Self::NativeContext,
        Self::DaSpec,
    >>::GenesisConfig {
        let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();

        #[cfg(feature = "experimental")]
        let eth_signer = read_eth_tx_signers();

        get_genesis_config(
            sequencer_da_address,
            genesis_paths,
            #[cfg(feature = "experimental")]
            eth_signer.signers(),
        )
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

    fn create_vm(&self) -> Self::Vm {
        Risc0Host::new(risc0::ROLLUP_ELF)
    }

    fn create_verifier(&self) -> <Self::DaService as DaService>::Verifier {
        CelestiaVerifier {
            rollup_namespace: ROLLUP_NAMESPACE,
        }
    }

    fn create_zk_storage(
        &self,
        _rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::ZkContext as Spec>::Storage {
        ZkStorage::new()
    }

    fn create_native_storage(
        &self,
        rollup_config: &sov_stf_runner::RollupConfig<Self::DaConfig>,
    ) -> Result<<Self::NativeContext as sov_modules_api::Spec>::Storage, anyhow::Error> {
        let storage_config = StorageConfig {
            path: rollup_config.storage.path.clone(),
        };
        ProverStorage::with_config(storage_config)
    }

    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as sov_modules_api::Spec>::Storage,
        ledger_db: &sov_db::ledger_db::LedgerDB,
        da_service: &Self::DaService,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
        create_rpc_methods(storage, ledger_db, da_service.clone())
    }
}
