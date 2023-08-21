use std::str::FromStr;

use anyhow::Context;
use celestia::verifier::address::CelestiaAddress;
use celestia::verifier::RollupParams;
use celestia::CelestiaService;
use const_rollup_config::SEQUENCER_DA_ADDRESS;
use demo_stf::app::{App, DefaultContext};
use demo_stf::runtime::{get_rpc_methods, GenesisConfig};
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::LedgerDB;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::Zkvm;
use sov_state::storage::Storage;
use sov_stf_runner::{from_toml_path, RollupConfig, RunnerConfig, StateTransitionRunner};
use tracing::debug;

#[cfg(feature = "experimental")]
use crate::register_rpc::register_ethereum;
use crate::register_rpc::{register_ledger, register_sequencer};
use crate::{get_genesis_config, initialize_ledger, ROLLUP_NAMESPACE};

/// Dependencies needed to run the rollup.
pub struct Rollup<Vm: Zkvm, DA: DaService + Clone> {
    ///
    pub app: App<Vm, DA::Spec>,
    ///
    pub da_service: DA,
    ///
    pub ledger_db: LedgerDB,
    ///
    pub runner_config: RunnerConfig,
    ///
    pub genesis_config: GenesisConfig<DefaultContext>,
}

/// Creates celestia based rollup.
pub async fn new_rollup_with_celestia_da(
    rollup_config_path: &str,
) -> Result<Rollup<Risc0Verifier, CelestiaService>, anyhow::Error> {
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
    let sequencer_da_address = CelestiaAddress::from_str(SEQUENCER_DA_ADDRESS).unwrap();
    let genesis_config = get_genesis_config(sequencer_da_address);

    Ok(Rollup {
        app,
        da_service,
        ledger_db,
        runner_config: rollup_config.runner,
        genesis_config,
    })
}

impl<Vm: Zkvm, DA: DaService<Error = anyhow::Error> + Clone> Rollup<Vm, DA> {
    /// Runs the rollup.
    pub async fn run(mut self) -> Result<(), anyhow::Error> {
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
        )?;

        runner.start_rpc_server(methods).await;
        runner.run().await?;

        Ok(())
    }
}
