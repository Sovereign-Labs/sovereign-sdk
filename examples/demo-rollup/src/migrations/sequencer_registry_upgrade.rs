//! Exports or verifies the complete sequencer registry at the DA rotation upgrade boundary.

use std::path::PathBuf;

use anyhow::{ensure, Context};
use clap::{Parser, ValueEnum};
use demo_stf::runtime::Runtime;
use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::{InitializableNativeNomtStorage, NomtStorageManager};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::{CryptoSpec, DaSpec, Spec, StateCheckpoint};
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_sequencer_registry::upgrade::{export_addresses, verify_addresses};
use sov_state::NativeStorage;

#[derive(Clone, Debug, ValueEnum)]
enum DaLayer {
    #[cfg(feature = "mock_da")]
    Mock,
    #[cfg(feature = "celestia_da")]
    Celestia,
}

#[derive(Debug, Parser)]
#[command(about = "Prepare DA rotation upgrade constants from a stopped node's finalized snapshot")]
struct Args {
    #[arg(long)]
    rollup_config_path: PathBuf,
    #[arg(long, value_enum)]
    da_layer: DaLayer,
    /// First execution height with DA rotation enabled; snapshot must be at H-1.
    #[arg(
        long,
        required_unless_present = "verify_config",
        conflicts_with = "verify_config"
    )]
    activation_height: Option<u64>,
    /// Verify the compiled activation height and address list against this snapshot.
    #[arg(long)]
    verify_config: bool,
}

type Hasher<S> = <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    match args.da_layer {
        #[cfg(feature = "mock_da")]
        DaLayer::Mock => {
            run::<<sov_demo_rollup::MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec>(&args)
        }
        #[cfg(feature = "celestia_da")]
        DaLayer::Celestia => run::<
            <sov_demo_rollup::CelestiaDemoRollup<Native> as RollupBlueprint<Native>>::Spec,
        >(&args),
    }
}

fn run<S>(args: &Args) -> anyhow::Result<()>
where
    S: Spec<Address = demo_stf::MultiAddressEvmSolana>,
    S::Storage: InitializableNativeNomtStorage<Hasher<S>, <S::Da as DaSpec>::SlotHash>,
{
    let activation_height = if args.verify_config {
        config_value!("SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT")
    } else {
        args.activation_height
            .context("activation height is required")?
    };
    ensure!(
        activation_height > 0,
        "height zero is for fresh chains and has no upgrade snapshot"
    );
    let mut config = sov_migrations::load_storage_config(
        &args.rollup_config_path,
        &sov_migrations::DbPathOverrides::default(),
    )?;
    config.pruner = sov_db::config::PrunerConfig::Off;
    let manager = NomtStorageManager::<S::Da, Hasher<S>, S::Storage>::new(config, false)?;
    let (storage, ledger_reader) = manager.create_state_for_migration()?;
    let ledger = LedgerDb::with_reader(ledger_reader)?;
    let (slot, head) = ledger
        .get_head_slot()?
        .context("snapshot has no ledger head")?;
    ensure!(
        storage.latest_version() == slot,
        "storage and ledger head slots differ"
    );
    let root = storage
        .get_root_hash(slot)
        .context("snapshot has no state root")?;
    ensure!(
        head.state_root.as_ref() == root.as_ref(),
        "storage and ledger head roots differ"
    );

    let mut runtime = Runtime::<S>::default();
    let mut state = StateCheckpoint::new(storage.clone(), &runtime.kernel());
    let height = runtime.chain_state.rollup_height(&mut state)?;
    ensure!(
        height.get() == activation_height - 1,
        "snapshot execution height {} must equal activation height {} minus one",
        height.get(),
        activation_height
    );
    let addresses = export_addresses(&runtime.sequencer_registry, &storage)?;
    if args.verify_config {
        verify_addresses(
            &runtime.sequencer_registry,
            &storage,
            config_value!("SEQUENCER_DA_ROTATION_UPGRADE_ADDRESSES"),
        )?;
    }
    println!("# Execution height: {height}");
    println!("# DA slot: {slot}");
    println!("# Source state root: {root}");
    println!("SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT = {activation_height}");
    println!("SEQUENCER_DA_ROTATION_UPGRADE_ADDRESSES = {addresses:?}");
    Ok(())
}
