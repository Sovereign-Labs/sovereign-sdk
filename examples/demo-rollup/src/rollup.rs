use std::net::SocketAddr;
use std::str::FromStr;

use anyhow::Context;
use celestia::verifier::address::CelestiaAddress;
use celestia::verifier::{CelestiaVerifier, RollupParams};
use celestia::CelestiaService;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::app::{create_zk_app_template, App, DefaultContext};
use demo_stf::runtime::{get_rpc_methods, GenesisConfig, Runtime};
use risc0_adapter::Risc0Vm;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::DaVerifier;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::{ProofSystem, ZkvmHost};
use sov_state::storage::Storage;
use sov_stf_runner::{
    from_toml_path, RollupConfig, RunnerConfig, StateTransitionRunner, StateTransitionVerifier,
};
use tokio::sync::oneshot;
use tracing::debug;

#[cfg(feature = "experimental")]
use crate::register_rpc::register_ethereum;
use crate::register_rpc::{register_ledger, register_sequencer};
use crate::{get_genesis_config, initialize_ledger, ROLLUP_NAMESPACE};

/// A verifier for the demo rollup
pub type AppVerifier<DA, Zk> = StateTransitionVerifier<
    AppTemplate<ZkDefaultContext, <DA as DaVerifier>::Spec, Zk, Runtime<ZkDefaultContext>>,
    DA,
    Zk,
>;

/// Dependencies needed to run the rollup.
pub struct Rollup<Vm: ProofSystem, DA: DaService + Clone> {
    /// Implementation of the STF.
    pub app: App<Vm::Host, DA::Spec>,
    /// The verifier for this rollup
    pub verifier: Option<(Vm::Host, AppVerifier<DA::Verifier, Vm::Guest>)>,
    /// Data availability service.
    pub da_service: DA,
    /// Ledger db.
    pub ledger_db: LedgerDB,
    /// Runner configuration.
    pub runner_config: RunnerConfig,
    /// Initial rollup configuration.
    pub genesis_config: GenesisConfig<DefaultContext>,
}

/// Creates celestia based rollup.
pub async fn new_rollup_with_celestia_da<Vm: ProofSystem>(
    rollup_config_path: &str,
    prover: Option<Vm::Host>,
) -> Result<Rollup<Vm, CelestiaService>, anyhow::Error> {
    debug!("Starting demo rollup with config {}", rollup_config_path);
    let rollup_config: RollupConfig<celestia::DaServiceConfig> =
        from_toml_path(rollup_config_path).context("Failed to read rollup configuration")?;

    let ledger_db = initialize_ledger(&rollup_config.storage.path);

    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: ROLLUP_NAMESPACE,
        },
    )
    .await;

    let app = App::new(rollup_config.storage);
    let verifier = prover.map(|p| {
        (
            p,
            AppVerifier::new(
                create_zk_app_template(),
                CelestiaVerifier {
                    rollup_namespace: ROLLUP_NAMESPACE,
                },
            ),
        )
    });
    let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS)?;
    let genesis_config = get_genesis_config(sequencer_da_address);

    Ok(Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
        verifier,
    })
}

impl<Vm: ProofSystem, DA: DaService<Error = anyhow::Error> + Clone> Rollup<Vm, DA> {
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
        let mut methods = get_rpc_methods::<DefaultContext>(storage);

        // register rpc methods
        {
            register_ledger(self.ledger_db.clone(), &mut methods)?;
            register_sequencer(self.da_service.clone(), &mut self.app, &mut methods)?;
            #[cfg(feature = "experimental")]
            register_ethereum(self.da_service.clone(), &mut methods)?;
        }

        let storage = self.app.get_storage();

        let mut runner = StateTransitionRunner::new(
            self.runner_config,
            self.da_service,
            self.ledger_db,
            self.app.stf,
            storage.is_empty(),
            self.genesis_config,
            self.verifier,
        )?;

        runner.start_rpc_server(methods, channel).await;
        runner.run().await?;

        Ok(())
    }
}
