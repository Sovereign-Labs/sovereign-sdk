//! Defines the rollup full node implementation, including logic for configuring
//! and starting the rollup node.

use std::path::PathBuf;

use async_trait::async_trait;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_api::Spec;
use sov_modules_rollup_template::RollupTemplate;
use sov_modules_stf_template::{Runtime as RuntimeTrait, SequencerOutcome, TxEffect};
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::mocks::{MockDaConfig, MockDaService, MockDaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_state::config::Config as StorageConfig;
use sov_state::{ProverStorage, Storage, ZkStorage};
use sov_stf_runner::RollupConfig;
use stf_starter::genesis_config::{get_genesis_config, GenesisPaths};
use stf_starter::{get_rpc_methods, Runtime};

use crate::register_sequencer::register_sequencer;

/// Rollup with MockDa.
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
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> <Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisConfig
    {
        get_genesis_config(rollup_config.da.sender_address, genesis_paths)
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Self::DaService {
        MockDaService::new(rollup_config.da.sender_address)
    }

    fn create_vm(&self) -> Self::Vm {
        Risc0Host::new(risc0_starter::MOCK_DA_ELF)
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
        let mut module = get_rpc_methods::<DefaultContext, Self::DaSpec>(storage.clone());

        module.merge(sov_ledger_rpc::server::rpc_module::<
            LedgerDB,
            SequencerOutcome<<DefaultContext as Spec>::Address>,
            TxEffect,
        >(ledger_db.clone())?)?;

        register_sequencer::<Self::DaService>(storage, da_service.clone(), &mut module)?;
        Ok(module)
    }
}
