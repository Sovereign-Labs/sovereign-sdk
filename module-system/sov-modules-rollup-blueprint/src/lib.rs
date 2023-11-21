#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod runtime_rpc;
mod wallet;
use std::net::SocketAddr;

use async_trait::async_trait;
pub use runtime_rpc::*;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::runtime::capabilities::Kernel;
use sov_modules_api::{Context, DaSpec, Spec};
use sov_modules_stf_blueprint::{Runtime as RuntimeTrait, StfBlueprint};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::storage::StorageManager;
use sov_rollup_interface::zk::ZkvmHost;
use sov_state::storage::NativeStorage;
use sov_state::Storage;
use sov_stf_runner::{ProverService, RollupConfig, RollupProverConfig, StateTransitionRunner};
use tokio::sync::oneshot;
pub use wallet::*;

/// This trait defines how to crate all the necessary dependencies required by a rollup.
#[async_trait]
pub trait RollupBlueprint: Sized + Send + Sync {
    /// Data Availability service.
    type DaService: DaService<Spec = Self::DaSpec, Error = anyhow::Error> + Clone + Send + Sync;
    /// A specification for the types used by a DA layer.
    type DaSpec: DaSpec + Send + Sync;
    /// Data Availability config.
    type DaConfig: Send + Sync;

    /// Host of a zkVM program.
    type Vm: ZkvmHost + Send;

    /// Context for Zero Knowledge environment.
    type ZkContext: Context;
    /// Context for Native environment.
    type NativeContext: Context;

    /// Manager for the native storage lifecycle.
    type StorageManager: StorageManager<
        NativeStorage = <Self::NativeContext as Spec>::Storage,
        NativeChangeSet = (),
    >;

    /// Runtime for the Zero Knowledge environment.
    type ZkRuntime: RuntimeTrait<Self::ZkContext, Self::DaSpec> + Default;
    /// Runtime for the Native environment.
    type NativeRuntime: RuntimeTrait<Self::NativeContext, Self::DaSpec> + Default + Send + Sync;

    /// The kernel for the native environment.
    type NativeKernel: Kernel<Self::NativeContext, Self::DaSpec> + Default + Send + Sync;
    /// The kernel for the Zero Knowledge environment.
    type ZkKernel: Kernel<Self::ZkContext, Self::DaSpec> + Default;

    /// Prover service.
    type ProverService: ProverService<
        StateRoot = <<Self::NativeContext as Spec>::Storage as Storage>::Root,
        Witness = <<Self::NativeContext as Spec>::Storage as Storage>::Witness,
        DaService = Self::DaService,
    >;

    /// Creates RPC methods for the rollup.
    fn create_rpc_methods(
        &self,
        storage: &<Self::NativeContext as Spec>::Storage,
        ledger_db: &LedgerDB,
        da_service: &Self::DaService,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error>;

    /// Creates GenesisConfig from genesis files.
    fn create_genesis_config(
        &self,
        genesis_paths: &<Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisPaths,
        _rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> anyhow::Result<
        <Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisConfig,
    > {
        <Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::genesis_config(
            genesis_paths,
        )
    }

    /// Creates instance of [`DaService`].
    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Self::DaService;

    /// Creates instance of [`ProverService`].
    async fn create_prover_service(
        &self,
        prover_config: RollupProverConfig,
        da_service: &Self::DaService,
    ) -> Self::ProverService;

    /// Creates instance of [`StorageManager`].
    /// Panics if initialization fails.
    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<Self::DaConfig>,
    ) -> Result<Self::StorageManager, anyhow::Error>;

    /// Creates instance of a LedgerDB.
    fn create_ledger_db(&self, rollup_config: &RollupConfig<Self::DaConfig>) -> LedgerDB {
        LedgerDB::with_path(&rollup_config.storage.path).expect("Ledger DB failed to open")
    }

    /// Creates a new rollup.
    async fn create_new_rollup(
        &self,
        genesis_paths: &<Self::NativeRuntime as RuntimeTrait<Self::NativeContext, Self::DaSpec>>::GenesisPaths,
        rollup_config: RollupConfig<Self::DaConfig>,
        prover_config: RollupProverConfig,
    ) -> Result<Rollup<Self>, anyhow::Error>
    where
        <Self::NativeContext as Spec>::Storage: NativeStorage,
    {
        let da_service = self.create_da_service(&rollup_config).await;
        let prover_service = self.create_prover_service(prover_config, &da_service).await;

        let ledger_db = self.create_ledger_db(&rollup_config);
        let genesis_config = self.create_genesis_config(genesis_paths, &rollup_config)?;

        let storage_manager = self.create_storage_manager(&rollup_config)?;
        let native_storage = storage_manager.get_native_storage();

        let prev_root = ledger_db
            .get_head_slot()?
            .map(|(number, _)| native_storage.get_root_hash(number.0))
            .transpose()?;

        let rpc_methods = self.create_rpc_methods(&native_storage, &ledger_db, &da_service)?;

        let native_stf = StfBlueprint::new();

        let runner = StateTransitionRunner::new(
            rollup_config.runner,
            da_service,
            ledger_db,
            native_stf,
            storage_manager,
            prev_root,
            genesis_config,
            prover_service,
        )?;

        Ok(Rollup {
            runner,
            rpc_methods,
        })
    }
}

/// Dependencies needed to run the rollup.
pub struct Rollup<S: RollupBlueprint> {
    /// The State Transition Runner.
    #[allow(clippy::type_complexity)]
    pub runner: StateTransitionRunner<
        StfBlueprint<S::NativeContext, S::DaSpec, S::Vm, S::NativeRuntime, S::NativeKernel>,
        S::StorageManager,
        S::DaService,
        S::Vm,
        S::ProverService,
    >,
    /// Rpc methods for the rollup.
    pub rpc_methods: jsonrpsee::RpcModule<()>,
}

impl<S: RollupBlueprint> Rollup<S> {
    /// Runs the rollup.
    pub async fn run(self) -> Result<(), anyhow::Error> {
        self.run_and_report_rpc_port(None).await
    }

    /// Runs the rollup. Reports rpc port to the caller using the provided channel.
    pub async fn run_and_report_rpc_port(
        self,
        channel: Option<oneshot::Sender<SocketAddr>>,
    ) -> Result<(), anyhow::Error> {
        let mut runner = self.runner;
        runner.start_rpc_server(self.rpc_methods, channel).await;
        runner.run_in_process().await?;
        Ok(())
    }
}
