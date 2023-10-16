//! Defines the rollup full node implementation, including logic for configuring
//! and starting the rollup node.
use serde::de::DeserializeOwned;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::{DefaultContext, ZkDefaultContext};
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::ZkvmHost;
use sov_stf_runner::{Prover, RollupConfig, RunnerConfig, StateTransitionRunner};
use template_stf::{get_rpc_methods, GenesisConfig, Runtime, StfWithBuilder};
use tokio::sync::oneshot;

use crate::rpc::{register_ledger, register_sequencer};

type ZkStf<Da, Vm> = AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>>;

/// Dependencies needed to run the rollup.
/// This is duplicated exactly from demo-rollup. Should go to stf-runner crate?
pub struct Rollup<Vm: ZkvmHost, Da: DaService + Clone> {
    // Implementation of the STF.
    pub(crate) app: StfWithBuilder<Vm, Da::Spec>,
    // Data availability service.
    pub(crate) da_service: Da,
    // Ledger db.
    pub(crate) ledger_db: LedgerDB,
    // Runner configuration.
    pub(crate) runner_config: RunnerConfig,
    // Initial rollup configuration.
    pub(crate) genesis_config: GenesisConfig<DefaultContext, Da::Spec>,
    // Prover for the rollup.
    #[allow(clippy::type_complexity)]
    pub(crate) prover: Option<Prover<ZkStf<Da::Spec, Vm::Guest>, Da, Vm>>,
}

impl<Vm: ZkvmHost, Da: DaService<Error = anyhow::Error> + Clone> Rollup<Vm, Da> {
    /// Creates a new rollup instance
    #[allow(clippy::type_complexity)]
    pub fn new<DaConfig: DeserializeOwned>(
        da_service: Da,
        genesis_config: GenesisConfig<DefaultContext, Da::Spec>,
        config: RollupConfig<DaConfig>,
        prover: Option<Prover<ZkStf<Da::Spec, Vm::Guest>, Da, Vm>>,
    ) -> Result<Self, anyhow::Error> {
        let ledger_db = LedgerDB::with_path(&config.storage.path)?;
        let app = StfWithBuilder::new(config.storage.clone());
        Ok(Self {
            app,
            da_service,
            ledger_db,
            runner_config: config.runner,
            genesis_config,
            prover,
        })
    }

    /// Runs the rollup.
    pub async fn run(self) -> Result<(), anyhow::Error> {
        self.run_and_report_rpc_port(None).await
    }

    /// Runs the rollup. Reports rpc port to the caller using the provided channel.
    pub async fn run_and_report_rpc_port(
        mut self,
        channel: Option<oneshot::Sender<std::net::SocketAddr>>,
    ) -> Result<(), anyhow::Error> {
        let storage = self.app.get_storage();
        let last_slot_opt = self.ledger_db.get_head_slot()?;
        let prev_root = last_slot_opt
            .map(|(number, _)| storage.get_root_hash(number.0))
            .transpose()?;
        let mut methods = get_rpc_methods::<DefaultContext, Da::Spec>(storage.clone());

        // register rpc methods
        {
            register_ledger::<Da>(self.ledger_db.clone(), &mut methods)?;
            register_sequencer(self.da_service.clone(), &mut self.app, &mut methods)?;
        }

        let mut runner = StateTransitionRunner::new(
            self.runner_config,
            self.da_service,
            self.ledger_db,
            self.app.stf,
            prev_root,
            self.genesis_config,
            self.prover,
        )?;

        runner.start_rpc_server(methods, channel).await;
        runner.run_in_process().await?;

        Ok(())
    }
}
