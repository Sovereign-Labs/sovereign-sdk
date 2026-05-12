//! Offline migration tool that upgrades demo-rollup state from version 0 to version 1.

use std::path::PathBuf;

use clap::Parser;
use demo_stf::runtime::Runtime;
use sov_demo_rollup::MockDemoRollup;
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, Spec};
use sov_modules_rollup_blueprint::RollupBlueprint;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Offline migration tool for demo-rollup state version 0 to version 1."
)]
struct Args {
    /// Path to the rollup config file used by the running node.
    #[arg(long)]
    rollup_config_path: PathBuf,

    /// Override `storage.path` from the rollup config.
    #[arg(long)]
    db_path: Option<PathBuf>,

    /// Compute the post-migration state root but do not commit changes.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

type RollupSpec = <MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type Hasher = <<RollupSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher;

fn main() {
    if let Err(err) = run() {
        eprintln!("migrate_to_v1 failed: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut runtime = Runtime::<RollupSpec>::default();
    let runtime_inner = &mut *runtime;
    sov_migrations::v1::run::<RollupSpec, Hasher, StorableMockDaService>(
        sov_migrations::MigrationArgs {
            rollup_config_path: args.rollup_config_path,
            db_path: args.db_path,
            dry_run: args.dry_run,
        },
        &mut runtime_inner.accounts,
        &mut runtime_inner.chain_state,
    )?;
    Ok(())
}
