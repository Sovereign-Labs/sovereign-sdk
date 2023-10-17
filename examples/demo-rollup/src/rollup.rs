use std::default;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

use anyhow::Context as _;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::genesis_config::{get_genesis_config, GenesisPaths, StorageConfig};
use demo_stf::runtime::{GenesisConfig, Runtime};
use demo_stf::{create_zk_app_template, App, AppVerifier};
use jsonrpsee::core::async_trait;
use jsonrpsee::RpcModule;
#[cfg(feature = "experimental")]
use secp256k1::SecretKey;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::{CelestiaVerifier, RollupParams};
use sov_celestia_adapter::CelestiaService;
#[cfg(feature = "experimental")]
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_db::ledger_db::LedgerDB;
#[cfg(feature = "experimental")]
use sov_ethereum::experimental::EthRpcConfig;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
#[cfg(feature = "experimental")]
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, DaSpec, Spec};
use sov_modules_stf_template::{AppTemplate, SequencerOutcome, TxEffect};
use sov_risc0_adapter::host::Risc0Host;

use sov_rollup_interface::mocks::{
    MockAddress, MockDaConfig, MockDaService, MockDaSpec, MOCK_SEQUENCER_DA_ADDRESS,
};

#[cfg(feature = "experimental")]
use crate::register_rpc::register_ethereum;
use crate::register_rpc::register_sequencer;
use crate::{initialize_ledger, ROLLUP_NAMESPACE};
use demo_stf::runtime::get_rpc_methods;
use demo_stf::{create_batch_builder, create_native_app_template, get_storage};
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::ZkvmHost;
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
use sov_state::{DefaultStorageSpec, ProverStorage};
use sov_stf_runner::{
    from_toml_path, ProofGenConfig, Prover, RollupConfig, RunnerConfig, StateTransitionRunner,
};
use tokio::sync::oneshot;
use tracing::debug;

#[cfg(feature = "experimental")]
const TX_SIGNER_PRIV_KEY_PATH: &str = "../test-data/keys/tx_signer_private_key.json";

use sov_modules_stf_template::Runtime as RuntimeTrait;
use sov_rollup_interface::stf::StateTransitionFunction;

type ZkStf<Da, Vm> = AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>>;

pub trait RollupSpec: Sized + Send + Sync {
    type DaService: DaService<Spec = Self::DaSpec, Error = anyhow::Error> + Clone;
    type DaSpec: DaSpec;
    type DaConfig;

    type ZkContext: Context;
    type DefaultContext: Context;

    type ZkRuntime: RuntimeTrait<Self::ZkContext, Self::DaSpec>;
    type NativeRuntime: RuntimeTrait<Self::DefaultContext, Self::DaSpec>;

    type Vm: ZkvmHost;
    type Builder: BatchBuilder + Send + Sync + 'static;

    type ZkSTF: StateTransitionFunction<<Self::Vm as ZkvmHost>::Guest, Self::DaSpec>;
    type NativeSTF: StateTransitionFunction<
        Self::Vm,
        Self::DaSpec,
        Condition = <Self::DaSpec as DaSpec>::ValidityCondition,
    >;

    fn create_methods(
        &self,
        rollup: &NewRollup<Self>,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error>;

    fn get_genesis_config<P: AsRef<Path>>(
        &self,
        genesis_paths: &GenesisPaths<P>,
    ) -> <Self::NativeRuntime as RuntimeTrait<Self::DefaultContext, Self::DaSpec>>::GenesisConfig;

    fn get_sequencer_da_address(&self) -> <Self::DaSpec as DaSpec>::Address;

    fn get_sequencer_da_service(&self) -> Self::DaService;

    fn get_prover(&self) -> Prover<Self::ZkSTF, Self::DaService, Self::Vm>;

    fn get_ledger_db(&self, rollup_config: RollupConfig<Self::DaConfig>) -> LedgerDB {
        initialize_ledger(&rollup_config.storage.path)
    }

    fn create_new_rollup<P: AsRef<Path>>(
        &self,
        genesis_paths: GenesisPaths<P>,
        rollup_config: RollupConfig<Self::DaConfig>,
    ) -> Result<NewRollup<Self>, anyhow::Error>;
}

pub struct DemoRollupSpec {}

impl RollupSpec for DemoRollupSpec {
    type DaService = MockDaService;
    type DaSpec = MockDaSpec;
    type DaConfig = MockDaConfig;

    type Vm = Risc0Host<'static>;

    type ZkContext = ZkDefaultContext;
    type DefaultContext = DefaultContext;
    type Builder = FiFoStrictBatchBuilder<Self::NativeRuntime, Self::DefaultContext>;

    ///
    type ZkRuntime = Runtime<Self::ZkContext, Self::DaSpec>;
    type ZkSTF =
        AppTemplate<Self::ZkContext, Self::DaSpec, <Self::Vm as ZkvmHost>::Guest, Self::ZkRuntime>;

    type NativeRuntime = Runtime<Self::DefaultContext, Self::DaSpec>;
    type NativeSTF = AppTemplate<Self::DefaultContext, Self::DaSpec, Self::Vm, Self::NativeRuntime>;

    fn create_methods(
        &self,
        rollup: &NewRollup<Self>,
    ) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error> {
        let batch_builder =
            create_batch_builder::<<DemoRollupSpec as RollupSpec>::DaSpec>(rollup.storage.clone());

        let mut methods = demo_stf::runtime::get_rpc_methods::<
            <DemoRollupSpec as RollupSpec>::DefaultContext,
            <DemoRollupSpec as RollupSpec>::DaSpec,
        >(rollup.storage.clone());

        methods.merge(
            sov_ledger_rpc::server::rpc_module::<
                LedgerDB,
                //TODO fix address
                SequencerOutcome<MockAddress>,
                TxEffect,
            >(rollup.ledger_db.clone())?
            .remove_context(),
        )?;

        register_seq(rollup.da_service.clone(), batch_builder, &mut methods)?;

        #[cfg(feature = "experimental")]
        let eth_signer = read_eth_tx_signers();

        #[cfg(feature = "experimental")]
        let eth_rpc_config = EthRpcConfig {
            min_blob_size: Some(1),
            sov_tx_signer_priv_key: read_sov_tx_signer_priv_key()?,
            eth_signer,
        };

        #[cfg(feature = "experimental")]
        register_ethereum::<
            <DemoRollupSpec as RollupSpec>::DefaultContext,
            <DemoRollupSpec as RollupSpec>::DaService,
        >(
            self.da_service.clone(),
            eth_rpc_config,
            self.storage.clone(),
            &mut methods,
        )
        .unwrap();

        Ok(methods)
    }

    fn get_genesis_config<P: AsRef<Path>>(
        &self,
        genesis_paths: &GenesisPaths<P>,
    ) -> <Self::NativeRuntime as RuntimeTrait<Self::DefaultContext, Self::DaSpec>>::GenesisConfig
    {
        let sequencer_da_address = MockAddress::from(self.get_sequencer_da_address());

        #[cfg(feature = "experimental")]
        let eth_signer = read_eth_tx_signers();

        get_genesis_config(
            sequencer_da_address,
            genesis_paths,
            #[cfg(feature = "experimental")]
            eth_signer.signers(),
        )
    }

    fn get_sequencer_da_address(&self) -> <Self::DaSpec as DaSpec>::Address {
        MockAddress::from(MOCK_SEQUENCER_DA_ADDRESS)
    }

    fn get_sequencer_da_service(&self) -> Self::DaService {
        MockDaService::new(self.get_sequencer_da_address())
    }

    // TODO change it to get vm & prover_config and assemble Prover in create_new_rollup
    fn get_prover(&self) -> Prover<Self::ZkSTF, Self::DaService, Self::Vm> {
        let vm = Risc0Host::new(risc0::MOCK_DA_ELF);
        let prover_config = ProofGenConfig::Execute;
        /*
                let prover_config = ProofGenConfig::Simulate(AppVerifier::new(
                    create_zk_app_template::<<<DemoRollupSpec as RollupSpec>::Vm as ZkvmHost>::Guest, _>(),
                    Default::default(),
                ));
        */
        Prover {
            vm,
            config: prover_config,
        }
    }

    fn create_new_rollup<P: AsRef<Path>>(
        &self,
        genesis_paths: GenesisPaths<P>,
        rollup_config: RollupConfig<MockDaConfig>,
    ) -> Result<NewRollup<Self>, anyhow::Error> {
        let ledger_db = self.get_ledger_db(rollup_config.clone());
        let genesis_config = self.get_genesis_config(&genesis_paths);

        let storage_config = StorageConfig {
            path: rollup_config.storage.path,
        };

        let prover = self.get_prover();
        let da_service = self.get_sequencer_da_service();

        let storage = get_storage(storage_config);
        let native_stf = create_native_app_template(storage.clone());

        let last_slot_opt = ledger_db.get_head_slot()?;

        let prev_root = last_slot_opt
            .map(|(number, _)| storage.get_root_hash(number.0))
            .transpose()?;

        //====
        let mut runner = StateTransitionRunner::new(
            rollup_config.runner,
            da_service.clone(),
            ledger_db.clone(),
            native_stf,
            prev_root,
            genesis_config,
            Some(prover),
        )?;

        Ok(NewRollup {
            storage,
            runner,
            ledger_db,
            da_service,
        })
    }
}

pub struct NewRollup<S: RollupSpec> {
    pub storage: <S::DefaultContext as Spec>::Storage,
    pub runner: StateTransitionRunner<S::NativeSTF, S::DaService, S::Vm, S::ZkSTF>,
    pub ledger_db: LedgerDB,
    pub da_service: S::DaService,
}

impl<S: RollupSpec> NewRollup<S> {
    /// Runs the rollup.
    pub async fn run(self, methods: jsonrpsee::RpcModule<()>) -> Result<(), anyhow::Error> {
        self.run_and_report_rpc_port(None, methods).await
    }

    /// Runs the rollup. Reports rpc port to the caller using the provided channel.
    pub async fn run_and_report_rpc_port(
        mut self,
        channel: Option<oneshot::Sender<SocketAddr>>,
        methods: jsonrpsee::RpcModule<()>,
    ) -> Result<(), anyhow::Error> {
        let mut runner = self.runner;
        runner.start_rpc_server(methods, channel).await;
        runner.run_in_process().await?;
        Ok(())
    }
}
use sov_sequencer::get_sequencer_rpc;
// register sequencer rpc methods.
pub(crate) fn register_seq<Da, B: BatchBuilder + Send + Sync + 'static>(
    da_service: Da,
    batch_builder: B,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    Da: DaService,
{
    let sequencer_rpc = get_sequencer_rpc(batch_builder, da_service);
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}

/// Dependencies needed to run the rollup.
pub struct Rollup<Vm: ZkvmHost, Da: DaService + Clone> {
    // Implementation of the STF.
    pub(crate) app: App<Vm, Da::Spec>,
    // Data availability service.
    pub(crate) da_service: Da,
    // Ledger db.
    pub(crate) ledger_db: LedgerDB,
    // Runner configuration.
    pub(crate) runner_config: RunnerConfig,
    // Initial rollup configuration.
    pub(crate) genesis_config: GenesisConfig<DefaultContext, Da::Spec>,
    #[cfg(feature = "experimental")]
    // Configuration for the Ethereum RPC.
    pub(crate) eth_rpc_config: EthRpcConfig,
    // Prover for the rollup.
    #[allow(clippy::type_complexity)]
    pub(crate) prover: Option<Prover<ZkStf<Da::Spec, Vm::Guest>, Da, Vm>>,
}

pub fn configure_prover<Vm: ZkvmHost, Da: DaService>(
    vm: Vm,
    cfg: DemoProverConfig,
    da_verifier: Da::Verifier,
) -> Prover<ZkStf<Da::Spec, Vm::Guest>, Da, Vm> {
    let config = match cfg {
        DemoProverConfig::Simulate => ProofGenConfig::Simulate(AppVerifier::new(
            create_zk_app_template::<Vm::Guest, _>(),
            da_verifier,
        )),
        DemoProverConfig::Execute => ProofGenConfig::Execute,
        DemoProverConfig::Prove => ProofGenConfig::Prover,
    };
    Prover { vm, config }
}

/// The possible configurations of the demo prover
pub enum DemoProverConfig {
    /// Run the rollup verification logic inside the current process
    Simulate,
    /// Run the rollup verifier in a zkVM executor
    Execute,
    /// Run the rollup verifier and create a SNARK of execution
    Prove,
}

/// Creates celestia based rollup.
pub async fn new_rollup_with_celestia_da<Vm: ZkvmHost, P: AsRef<Path>>(
    rollup_config_path: &str,
    prover: Option<(Vm, DemoProverConfig)>,
    genesis_paths: &GenesisPaths<P>,
) -> Result<Rollup<Vm, CelestiaService>, anyhow::Error> {
    debug!(
        "Starting demo celestia rollup with config {}",
        rollup_config_path
    );

    let rollup_config: RollupConfig<sov_celestia_adapter::DaServiceConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let ledger_db = initialize_ledger(&rollup_config.storage.path);

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;

    let storage_config = StorageConfig {
        path: rollup_config.storage.path,
    };
    let app = App::new(storage_config);
    let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS)?;

    #[cfg(feature = "experimental")]
    let eth_signer = read_eth_tx_signers();
    let genesis_config = get_genesis_config(
        sequencer_da_address,
        genesis_paths,
        #[cfg(feature = "experimental")]
        eth_signer.signers(),
    );
    let prover = prover.map(|(vm, config)| {
        configure_prover(
            vm,
            config,
            CelestiaVerifier {
                rollup_namespace: ROLLUP_NAMESPACE,
            },
        )
    });

    Ok(Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
        #[cfg(feature = "experimental")]
        eth_rpc_config: EthRpcConfig {
            min_blob_size: Some(1),
            sov_tx_signer_priv_key: read_sov_tx_signer_priv_key()?,
            eth_signer,
        },
        prover,
    })
}

/// Creates MockDa based rollup.
pub fn new_rollup_with_mock_da<Vm: ZkvmHost, P: AsRef<Path>>(
    rollup_config_path: &str,
    prover: Option<(Vm, DemoProverConfig)>,
    genesis_paths: &GenesisPaths<P>,
) -> Result<Rollup<Vm, MockDaService>, anyhow::Error> {
    debug!("Starting mock rollup with config {}", rollup_config_path);

    let rollup_config: RollupConfig<MockDaConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    new_rollup_with_mock_da_from_config(rollup_config, prover, genesis_paths)
}

/// Creates MockDa based rollup.
pub fn new_rollup_with_mock_da_from_config<Vm: ZkvmHost, P: AsRef<Path>>(
    rollup_config: RollupConfig<MockDaConfig>,
    prover: Option<(Vm, DemoProverConfig)>,
    genesis_paths: &GenesisPaths<P>,
) -> Result<Rollup<Vm, MockDaService>, anyhow::Error> {
    let ledger_db = initialize_ledger(&rollup_config.storage.path);
    let sequencer_da_address = MockAddress::from(MOCK_SEQUENCER_DA_ADDRESS);
    let da_service = MockDaService::new(sequencer_da_address);

    let storage_config = StorageConfig {
        path: rollup_config.storage.path,
    };
    let app = App::new(storage_config);

    #[cfg(feature = "experimental")]
    let eth_signer = read_eth_tx_signers();

    let genesis_config = get_genesis_config(
        sequencer_da_address,
        genesis_paths,
        #[cfg(feature = "experimental")]
        eth_signer.signers(),
    );

    let prover = prover.map(|(vm, config)| configure_prover(vm, config, Default::default()));
    Ok(Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
        #[cfg(feature = "experimental")]
        eth_rpc_config: EthRpcConfig {
            min_blob_size: Some(1),
            sov_tx_signer_priv_key: read_sov_tx_signer_priv_key()?,
            eth_signer,
        },
        prover,
    })
}

#[cfg(feature = "experimental")]
/// Ethereum RPC wraps EVM transaction in a rollup transaction.
/// This function reads the private key of the rollup transaction signer.
fn read_sov_tx_signer_priv_key() -> Result<DefaultPrivateKey, anyhow::Error> {
    let data = std::fs::read_to_string(TX_SIGNER_PRIV_KEY_PATH).context("Unable to read file")?;

    let key_and_address: PrivateKeyAndAddress<DefaultContext> = serde_json::from_str(&data)
        .unwrap_or_else(|_| panic!("Unable to convert data {} to PrivateKeyAndAddress", &data));

    Ok(key_and_address.private_key)
}

// TODO: #840
#[cfg(feature = "experimental")]
fn read_eth_tx_signers() -> sov_ethereum::DevSigner {
    sov_ethereum::DevSigner::new(vec![SecretKey::from_str(
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .unwrap()])
}

impl<Vm: ZkvmHost, Da: DaService<Error = anyhow::Error> + Clone> Rollup<Vm, Da> {
    /// Runs the rollup.
    pub async fn run(self) -> Result<(), anyhow::Error> {
        self.run_and_report_rpc_port(None).await
    }

    /// Runs the rollup. Reports rpc port to the caller using the provided channel.
    pub async fn run_and_report_rpc_port(
        mut self,
        channel: Option<oneshot::Sender<SocketAddr>>,
    ) -> Result<(), anyhow::Error> {
        let storage = self.app.get_storage();
        let last_slot_opt = self.ledger_db.get_head_slot()?;
        let prev_root = last_slot_opt
            .map(|(number, _)| storage.get_root_hash(number.0))
            .transpose()?;

        let rpc_module = self.rpc_module(storage)?;

        let mut runner = StateTransitionRunner::new(
            self.runner_config,
            self.da_service,
            self.ledger_db,
            self.app.stf,
            prev_root,
            self.genesis_config,
            self.prover,
        )?;

        runner.start_rpc_server(rpc_module, channel).await;
        runner.run_in_process().await?;

        Ok(())
    }

    /// Creates a new [`jsonrpsee::RpcModule`] and registers all RPC methods
    /// exposed by the node.
    fn rpc_module(
        &mut self,
        storage: <DefaultContext as Spec>::Storage,
    ) -> anyhow::Result<RpcModule<()>> {
        let mut module =
            demo_stf::runtime::get_rpc_methods::<DefaultContext, Da::Spec>(storage.clone());

        module.merge(
            sov_ledger_rpc::server::rpc_module::<
                LedgerDB,
                SequencerOutcome<<<Da as DaService>::Spec as DaSpec>::Address>,
                TxEffect,
            >(self.ledger_db.clone())?
            .remove_context(),
        )?;
        register_sequencer(self.da_service.clone(), &mut self.app, &mut module)?;
        #[cfg(feature = "experimental")]
        {
            register_ethereum::<DefaultContext, Da>(
                self.da_service.clone(),
                self.eth_rpc_config.clone(),
                storage,
                &mut module,
            )?;
            println!("Registered ethereum rpc");
        }

        Ok(module)
    }
}
