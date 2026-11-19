//! Offline migration tool that upgrades demo-rollup state from version 0 to version 1.

use std::path::PathBuf;

use clap::Parser;
use demo_stf::runtime::Runtime;
use sov_demo_rollup::MockDemoRollup;
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

    /// Override `storage.live_db_path` from the rollup config.
    #[arg(long)]
    live_db_path: Option<PathBuf>,

    /// Override `storage.ledger_db_path` from the rollup config.
    #[arg(long)]
    ledger_db_path: Option<PathBuf>,

    /// Override `storage.user_nomt_db_path` from the rollup config.
    #[arg(long)]
    user_nomt_db_path: Option<PathBuf>,

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
    sov_migrations::v1::run::<RollupSpec, Hasher>(
        sov_migrations::MigrationArgs {
            rollup_config_path: args.rollup_config_path,
            db_paths: sov_migrations::DbPathOverrides {
                db_path: args.db_path,
                live_db_path: args.live_db_path,
                ledger_db_path: args.ledger_db_path,
                user_nomt_db_path: args.user_nomt_db_path,
            },
            dry_run: args.dry_run,
        },
        &mut runtime_inner.accounts,
        &mut runtime_inner.chain_state,
    )?;
    Ok(())
}
