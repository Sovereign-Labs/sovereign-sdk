//! Migration from state version 0 to state version 1.

use anyhow::Context;
use serde::Serialize;
use sov_accounts::{Account, Accounts};
use sov_chain_state::ChainState;
use sov_modules_api::{
    AccessoryStateValue, CredentialId, ModuleInfo, Spec, StateCheckpoint, StateWriter,
};
use sov_state::{BorshCodec, Kernel, NativeStorage, Prefix, SlotKey, SlotValue, StateUpdate};

use crate::MigrationStorage as _;

/// The state version expected before applying this migration.
pub const SOURCE_STATE_VERSION: u64 = 0;
/// The state version written after applying this migration.
pub const TARGET_STATE_VERSION: u64 = 1;

/// Data collected before the migration checkpoint consumes storage.
pub struct MigrationData<S: Spec> {
    legacy_accounts: Vec<(CredentialId, Account<S>)>,
    kernel_roundtrip: KernelRoundtrip,
}

struct KernelRoundtrip {
    key: SlotKey,
    value: SlotValue,
}

/// Summary of the state-version-0 to state-version-1 migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    /// State version read before the migration.
    pub from_state_version: u64,
    /// State version written by the migration.
    pub to_state_version: u64,
    /// Summary of the sov-accounts sub-migration.
    pub accounts: sov_accounts::migrations::MigrationReport,
}

/// Collects all data needed for the v1 migration before storage is moved into a checkpoint.
pub fn collect<S, Storage>(
    accounts: &Accounts<S>,
    chain_state: &ChainState<S>,
    storage: &Storage,
) -> anyhow::Result<MigrationData<S>>
where
    S: Spec,
    Storage: NativeStorage,
{
    let legacy_accounts =
        sov_accounts::migrations::collect_legacy_account_entries(accounts, storage)
            .context("failed to collect legacy accounts entries")?;
    let kernel_roundtrip = collect_kernel_roundtrip(chain_state, storage)?;

    Ok(MigrationData {
        legacy_accounts,
        kernel_roundtrip,
    })
}

/// Applies the v1 migration to a checkpoint.
pub fn apply<S>(
    accounts: &mut Accounts<S>,
    chain_state: &mut ChainState<S>,
    data: MigrationData<S>,
    state: &mut StateCheckpoint<S>,
) -> anyhow::Result<MigrationReport>
where
    S: Spec,
{
    let from_state_version = {
        let mut accessory_state = state.accessory_state();
        chain_state
            .state_version(&mut accessory_state)
            .context("failed to read chain-state state_version")?
    };
    anyhow::ensure!(
        from_state_version == SOURCE_STATE_VERSION,
        "cannot apply v1 migration: on-disk state_version is {from_state_version}, expected {SOURCE_STATE_VERSION}"
    );

    let accounts_report = sov_accounts::migrations::apply_legacy_account_migration(
        accounts,
        &data.legacy_accounts,
        state,
    )
    .context("failed to apply legacy-accounts migration")?;
    {
        let mut accessory_state = state.accessory_state();
        state_version_value(chain_state)
            .set(&TARGET_STATE_VERSION, &mut accessory_state)
            .context("failed to update chain-state state_version")?;
    }
    StateWriter::<Kernel>::set(
        state,
        &data.kernel_roundtrip.key,
        data.kernel_roundtrip.value,
    )
    .context("failed to round-trip kernel value for historical-state invariant")?;

    Ok(MigrationReport {
        from_state_version,
        to_state_version: TARGET_STATE_VERSION,
        accounts: accounts_report,
    })
}

/// Runs the v1 migration and writes the JSON report to stdout.
pub fn run<S, H>(
    args: crate::MigrationArgs,
    accounts: &mut Accounts<S>,
    chain_state: &mut ChainState<S>,
) -> anyhow::Result<()>
where
    S: Spec,
    H: sov_rollup_interface::reexports::digest::Digest<
            OutputSize = sov_rollup_interface::reexports::digest::typenum::U32,
        > + Send
        + Sync,
    S::Storage: sov_db::storage_manager::InitializableNativeNomtStorage<
            H,
            <S::Da as sov_rollup_interface::da::DaSpec>::SlotHash,
        > + crate::MigrationStorage,
{
    let storage = crate::load_storage_config(&args.rollup_config_path, args.db_path.as_deref())?;
    let report = run_with_options::<S, H>(
        crate::MigrationOptions {
            storage,
            dry_run: args.dry_run,
        },
        accounts,
        chain_state,
    )?;
    let json = serde_json::to_string_pretty(&report)
        .context("failed to serialize migration report JSON")?;
    println!("{json}");
    Ok(())
}

/// Runs the v1 migration with an already-loaded storage config.
pub fn run_with_options<S, H>(
    options: crate::MigrationOptions,
    accounts: &mut Accounts<S>,
    chain_state: &mut ChainState<S>,
) -> anyhow::Result<crate::MigrationOutcome<MigrationReport>>
where
    S: Spec,
    H: sov_rollup_interface::reexports::digest::Digest<
            OutputSize = sov_rollup_interface::reexports::digest::typenum::U32,
        > + Send
        + Sync,
    S::Storage: sov_db::storage_manager::InitializableNativeNomtStorage<
            H,
            <S::Da as sov_rollup_interface::da::DaSpec>::SlotHash,
        > + crate::MigrationStorage,
{
    let db_path = options.storage.path.clone();
    let mut storage_manager =
        sov_db::storage_manager::NomtStorageManager::<S::Da, H, S::Storage>::new(
            options.storage,
            false,
        )
        .with_context(|| format!("failed to open storage manager at {}", db_path.display()))?;
    let (storage, ledger_reader) = storage_manager
        .create_state_for_migration()
        .context("failed to create migration storage view")?;
    let ledger_db = sov_db::ledger_db::LedgerDb::with_reader(ledger_reader)
        .context("failed to initialize ledger db")?;

    let head_slot_number = crate::assert_storage_latest_version_matches_ledger_head(
        &storage,
        &ledger_db,
        "pre-migration",
    )?;
    crate::assert_ledger_head_state_root_matches_storage_root(
        &storage,
        &ledger_db,
        "pre-migration",
    )?;

    let pre_state_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read pre-migration state root")?;
    let migration_data = collect(accounts, chain_state, &storage)?;

    let mut checkpoint = {
        let kernel = crate::MigrationKernel {
            chain_state: &*chain_state,
        };
        sov_modules_api::StateCheckpoint::new(storage, &kernel)
    };
    let migration_report = apply(accounts, chain_state, migration_data, &mut checkpoint)?;

    let (next_state_root, mut state_update, accessory_delta, _witness, storage_after) =
        checkpoint.materialize_update(pre_state_root.clone());
    state_update.add_accessory_items(accessory_delta.freeze());
    let change_set =
        storage_after.materialize_migration_changes_at_version(state_update, head_slot_number);

    let post_state_root = if options.dry_run {
        next_state_root
    } else {
        let ledger_change_set =
            crate::make_ledger_root_patch(&ledger_db, next_state_root.as_ref())?;
        storage_manager
            .commit_migration_change_set_at_head(head_slot_number, change_set, ledger_change_set)
            .context("failed to commit migration changeset at head version")?;

        let (post_storage, post_ledger_reader) = storage_manager
            .create_state_for_migration()
            .context("failed to create post-migration storage view")?;
        let post_ledger_db = sov_db::ledger_db::LedgerDb::with_reader(post_ledger_reader)
            .context("failed to initialize post-migration ledger db view")?;
        let post_head_slot_number = crate::assert_storage_latest_version_matches_ledger_head(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;
        if post_head_slot_number != head_slot_number {
            anyhow::bail!(
                "post-migration head slot changed unexpectedly: expected {head_slot_number}, found {post_head_slot_number}"
            );
        }
        crate::assert_ledger_head_state_root_matches_storage_root(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;
        post_storage
            .get_root_hash(post_head_slot_number)
            .context("failed to read post-migration state root")?
    };

    Ok(crate::MigrationOutcome {
        dry_run: options.dry_run,
        db_path: db_path.display().to_string(),
        pre_state_root: pre_state_root.to_string(),
        post_state_root: post_state_root.to_string(),
        head_rollup_slot: head_slot_number.get(),
        migration: migration_report,
    })
}

fn collect_kernel_roundtrip<S, Storage>(
    chain_state: &ChainState<S>,
    storage: &Storage,
) -> anyhow::Result<KernelRoundtrip>
where
    S: Spec,
    Storage: NativeStorage,
{
    let key = SlotKey::singleton(&Prefix::new(
        chain_state.discriminant(),
        ChainState::<S>::TRUE_SLOT_NUMBER_ITEM_DISCRIMINANT,
    ));
    let value = storage.get_unbound::<Kernel>(key.clone()).ok_or_else(|| {
        anyhow::anyhow!(
            "kernel `chain_state.true_slot_number` is unset; cannot perform no-op kernel write"
        )
    })?;

    Ok(KernelRoundtrip { key, value })
}

fn state_version_value<S: Spec>(chain_state: &ChainState<S>) -> AccessoryStateValue<u64> {
    AccessoryStateValue::with_codec(
        Prefix::new(
            chain_state.discriminant(),
            ChainState::<S>::STATE_VERSION_ITEM_DISCRIMINANT,
        ),
        BorshCodec,
    )
}
