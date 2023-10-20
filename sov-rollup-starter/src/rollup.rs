//! Defines the rollup full node implementation, including logic for configuring
//! and starting the rollup node.

use async_trait::async_trait;
use jsonrpsee::RpcModule;
use serde::de::DeserializeOwned;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::RollupTemplate;
use sov_modules_stf_template::Runtime as RuntimeTrait;
use sov_modules_stf_template::{AppTemplate, SequencerOutcome, TxEffect};
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::mocks::MockDaConfig;
use sov_rollup_interface::mocks::MockDaService;
use sov_rollup_interface::mocks::MockDaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::config::Config as StorageConfig;
use sov_state::storage::NativeStorage;
use sov_state::ProverStorage;
use sov_state::Storage;
use sov_state::ZkStorage;
use sov_stf_runner::{Prover, RollupConfig, RunnerConfig, StateTransitionRunner};
use std::path::PathBuf;
use stf_starter::{get_genesis_config, GenesisPaths};
use stf_starter::{get_rpc_methods, GenesisConfig, Runtime, StfWithBuilder};
use tokio::sync::oneshot;

///TODO
pub struct StarterRollup {}

#[async_trait]
impl RollupTemplate for StarterRollup {
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
    ) -> <Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisConfig
    {
        let sequencer_da_address = todo!();
        get_genesis_config(sequencer_da_address, genesis_paths)
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Self::DaService {
        MockDaService::new(rollup_config.da.sender_address)
    }

    fn create_vm(&self) -> Self::Vm {
        //Risc0Host::new(risc0::MOCK_DA_ELF)
        todo!()
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
    ) -> <Self::NativeContext as Spec>::Storage {
        let storage_config = StorageConfig {
            path: rollup_config.storage.path.clone(),
        };
        ProverStorage::with_config(storage_config).expect("Failed to open prover storage")
    }

    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as Spec>::Storage,
        ledger_db: &LedgerDB,
        da_service: &Self::DaService,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
        //create_rpc_methods(storage, ledger_db, da_service.clone())
        todo!()
    }
}
