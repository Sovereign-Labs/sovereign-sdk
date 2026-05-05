//! Offline migration tool that copies every legacy
//! `sov_accounts::Accounts::accounts` entry into `Accounts::account_owners`
//! and deletes the source row.
//!
//! Run this against a stopped MockDA demo-rollup DB before deploying a binary
//! that has dropped the layer-1 `accounts` reads. The tool is structurally
//! identical for the Celestia rollup; swap `MockDemoRollup` for
//! `CelestiaDemoRollup` (and `MockDaSpec` for `CelestiaSpec`).

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::Parser;
use demo_stf::runtime::Runtime;
use rockbound::SchemaBatch;
use serde::Serialize;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::tables::SlotByNumber;
use sov_db::storage_manager::NomtStorageManager;
use sov_full_node_configs::runner::from_toml_path;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::MockDaSpec;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{Spec, StateCheckpoint};
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_rollup_interface::common::SlotNumber;
use sov_state::{NativeStorage, StateUpdate};
use sov_stf_runner::RollupConfig;

use sov_demo_rollup::MockDemoRollup;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Offline migration tool for sov-accounts: copies legacy `accounts` map entries to \
             `account_owners` and deletes the source rows."
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

    /// Optional path to write the JSON migration report.
    #[arg(long)]
    report_out: Option<PathBuf>,
}

type RollupSpec = <MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type Hasher = <<RollupSpec as Spec>::CryptoSpec as sov_modules_api::CryptoSpec>::Hasher;
type DemoStorage = <RollupSpec as Spec>::Storage;
type DemoStorageManager = NomtStorageManager<MockDaSpec, Hasher, DemoStorage>;

#[derive(Serialize)]
struct MigrationReport {
    dry_run: bool,
    db_path: String,
    pre_state_root: String,
    post_state_root: String,
    head_rollup_slot: u64,
    entries_migrated: u64,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("legacy-accounts migration failed: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    let storage_config = load_storage_config(&args)?;
    let db_path = storage_config.path.clone();

    let mut storage_manager = DemoStorageManager::new(storage_config, false)
        .with_context(|| format!("failed to open storage manager at {}", db_path.display()))?;
    let (storage, ledger_reader) = storage_manager
        .create_state_for_migration()
        .context("failed to create migration storage view")?;
    let ledger_db =
        LedgerDb::with_reader(ledger_reader).context("failed to initialize ledger db")?;

    let head_slot_number =
        assert_storage_latest_version_matches_ledger_head(&storage, &ledger_db, "pre-migration")?;
    assert_ledger_head_state_root_matches_storage_root(&storage, &ledger_db, "pre-migration")?;

    let pre_state_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read pre-migration state root")?;

    let mut runtime = Runtime::<RollupSpec>::default();
    let entries =
        sov_accounts::migrations::collect_legacy_account_entries(&runtime.accounts, &storage)
            .context("failed to collect legacy accounts entries")?;

    let mut checkpoint = StateCheckpoint::new(storage, &runtime.kernel(), None);
    let report = sov_accounts::migrations::apply_legacy_account_migration(
        &mut runtime.accounts,
        &entries,
        &mut checkpoint,
    )
    .context("failed to apply legacy-accounts migration to checkpoint")?;

    let (next_state_root, mut state_update, accessory_delta, _witness, storage_after) =
        checkpoint.materialize_update(pre_state_root.clone());
    state_update.add_accessory_items(accessory_delta.freeze());
    let change_set = storage_after.materialize_changes_at_version(state_update, head_slot_number);

    let post_state_root = if args.dry_run {
        next_state_root
    } else {
        let ledger_change_set = make_ledger_root_patch(&ledger_db, next_state_root.as_ref())?;
        storage_manager
            .commit_migration_change_set_at_head(head_slot_number, change_set, ledger_change_set)
            .context("failed to commit migration changeset at head version")?;

        let (post_storage, post_ledger_reader) = storage_manager
            .create_state_for_migration()
            .context("failed to create post-migration storage view")?;
        let post_ledger_db = LedgerDb::with_reader(post_ledger_reader)
            .context("failed to initialize post-migration ledger db view")?;
        let post_head_slot_number = assert_storage_latest_version_matches_ledger_head(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;
        if post_head_slot_number != head_slot_number {
            bail!(
                "post-migration head slot changed unexpectedly: expected {}, found {}",
                head_slot_number,
                post_head_slot_number
            );
        }
        assert_ledger_head_state_root_matches_storage_root(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;
        post_storage
            .get_root_hash(post_head_slot_number)
            .context("failed to read post-migration state root")?
    };

    let migration_report = MigrationReport {
        dry_run: args.dry_run,
        db_path: db_path.display().to_string(),
        pre_state_root: pre_state_root.to_string(),
        post_state_root: post_state_root.to_string(),
        head_rollup_slot: head_slot_number.get(),
        entries_migrated: report.entries_migrated,
    };

    let json = serde_json::to_string_pretty(&migration_report)
        .context("failed to serialize migration report JSON")?;

    if let Some(path) = args.report_out {
        std::fs::write(&path, &json)
            .with_context(|| format!("failed to write migration report to {}", path.display()))?;
    }

    println!("{json}");
    Ok(())
}

fn load_storage_config(args: &Args) -> anyhow::Result<sov_db::config::RollupDbConfig> {
    let rollup_config: RollupConfig<<RollupSpec as Spec>::Address, StorableMockDaService> =
        from_toml_path(&args.rollup_config_path).with_context(|| {
            format!(
                "failed to read rollup config from {}",
                args.rollup_config_path.display()
            )
        })?;

    let mut storage = rollup_config.storage;
    if let Some(db_path_override) = &args.db_path {
        storage.path = db_path_override.clone();
    }
    Ok(storage)
}

fn assert_storage_latest_version_matches_ledger_head<S: NativeStorage>(
    storage: &S,
    ledger_db: &LedgerDb,
    phase: &str,
) -> anyhow::Result<SlotNumber> {
    let (head_slot_number, _head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot migrate an empty DB"))?;
    let storage_latest_version = storage.latest_version();
    if storage_latest_version != head_slot_number {
        bail!(
            "{phase} invariant failed: storage.latest_version ({}) != ledger head slot ({})",
            storage_latest_version,
            head_slot_number
        );
    }
    Ok(head_slot_number)
}

fn assert_ledger_head_state_root_matches_storage_root<S: NativeStorage>(
    storage: &S,
    ledger_db: &LedgerDb,
    phase: &str,
) -> anyhow::Result<()> {
    let (head_slot_number, head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot migrate an empty DB"))?;
    let storage_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read storage root at ledger head slot")?;
    if head_slot.state_root.as_ref() != storage_root.as_ref() {
        bail!(
            "{phase} invariant failed: ledger head state_root does not match storage root at slot {}",
            head_slot_number
        );
    }
    Ok(())
}

fn make_ledger_root_patch(
    ledger_db: &LedgerDb,
    new_state_root: &[u8],
) -> anyhow::Result<SchemaBatch> {
    let (head_slot_number, mut head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot patch state root"))?;

    head_slot.state_root = new_state_root.to_vec().into();

    let mut batch = SchemaBatch::new();
    batch.put::<SlotByNumber>(&head_slot_number, &head_slot)?;
    Ok(batch)
}
