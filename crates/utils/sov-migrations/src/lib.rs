//! Reusable offline state migrations for Sovereign SDK rollups.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use rockbound::SchemaBatch;
use serde::Serialize;
use sov_chain_state::ChainState;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::tables::SlotByNumber;
use sov_db::storage_manager::NomtChangeSet;
use sov_modules_api::capabilities::{BlockGasInfo, RollupHeight};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::runtime::capabilities::Kernel as KernelTrait;
use sov_modules_api::{BootstrapWorkingSet, Spec, StateCheckpoint};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_rollup_interface::node::da::DaService;
use sov_state::NativeStorage;
use sov_stf_runner::{from_toml_path, RollupConfig};

/// CLI-shaped inputs for running a migration against a rollup database.
#[derive(Debug, Clone)]
pub struct MigrationArgs {
    /// Path to the rollup config file used by the running node.
    pub rollup_config_path: PathBuf,
    /// Override `storage.path` from the rollup config.
    pub db_path: Option<PathBuf>,
    /// Compute the post-migration state root but do not commit changes.
    pub dry_run: bool,
    /// Optional path to write the JSON migration report.
    pub report_out: Option<PathBuf>,
}

/// NOMT storage operations needed to rewrite an existing head version.
pub trait MigrationStorage: NativeStorage<ChangeSet = NomtChangeSet> {
    /// Materializes a state update at the current ledger-head version.
    fn materialize_migration_changes_at_version(
        self,
        state_update: Self::StateUpdate,
        version: SlotNumber,
    ) -> NomtChangeSet;
}

impl<S, K> MigrationStorage for sov_state::nomt::prover_storage::NomtProverStorage<S, K>
where
    S: sov_state::MerkleProofSpec,
    K: Clone + Eq + std::hash::Hash,
{
    fn materialize_migration_changes_at_version(
        self,
        state_update: Self::StateUpdate,
        version: SlotNumber,
    ) -> NomtChangeSet {
        self.materialize_changes_at_version(state_update, version)
    }
}

struct MigrationKernel<'a, S: Spec> {
    chain_state: &'a ChainState<S>,
}

impl<S: Spec> KernelTrait<S> for MigrationKernel<'_, S> {
    fn true_slot_number(&self, state: &mut BootstrapWorkingSet<'_, S>) -> SlotNumber {
        self.chain_state.true_slot_number_at_bootstrap(state)
    }

    fn next_visible_slot_number(
        &self,
        state: &mut BootstrapWorkingSet<'_, S>,
    ) -> VisibleSlotNumber {
        self.chain_state.next_visible_slot_number(state)
    }

    fn rollup_height(&self, state: &mut BootstrapWorkingSet<'_, S>) -> RollupHeight {
        self.chain_state.rollup_height(state).unwrap_infallible()
    }

    fn record_gas_usage(
        &mut self,
        _state: &mut StateCheckpoint<S>,
        _final_gas_info: BlockGasInfo<S::Gas>,
        _rollup_height: RollupHeight,
    ) {
        unreachable!("offline migration checkpoints do not record block gas usage")
    }
}

/// Migration from state version 0 to state version 1.
pub mod v1 {
    use anyhow::Context;
    use serde::Serialize;
    use sov_accounts::{Account, Accounts};
    use sov_chain_state::ChainState;
    use sov_modules_api::{CredentialId, ModuleInfo, Spec, StateReader, StateWriter};
    use sov_state::{Kernel, NativeStorage, Prefix, SlotKey, SlotValue, StateUpdate, User};

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
    pub fn apply<S, Writer>(
        accounts: &mut Accounts<S>,
        chain_state: &mut ChainState<S>,
        data: MigrationData<S>,
        state: &mut Writer,
    ) -> anyhow::Result<MigrationReport>
    where
        S: Spec,
        Writer: StateReader<User> + StateWriter<User> + StateWriter<Kernel>,
    {
        let from_state_version = chain_state
            .state_version(state)
            .context("failed to read chain-state state_version")?;
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
        chain_state
            .set_state_version(TARGET_STATE_VERSION, state)
            .context("failed to update chain-state state_version")?;
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

    /// Runs the v1 migration and returns the migration report.
    pub fn run<S, H, Da>(
        args: crate::MigrationArgs,
        accounts: &mut Accounts<S>,
        chain_state: &mut ChainState<S>,
    ) -> anyhow::Result<crate::MigrationOutcome<MigrationReport>>
    where
        S: Spec,
        H: sov_rollup_interface::reexports::digest::Digest<
                OutputSize = sov_rollup_interface::reexports::digest::typenum::U32,
            > + Send
            + Sync,
        Da: sov_rollup_interface::node::da::DaService<Spec = S::Da>,
        sov_stf_runner::RollupConfig<S::Address, Da>: serde::de::DeserializeOwned,
        S::Storage: sov_db::storage_manager::InitializableNativeNomtStorage<
                H,
                <S::Da as sov_rollup_interface::da::DaSpec>::SlotHash,
            > + crate::MigrationStorage,
    {
        let storage =
            crate::load_storage_config::<S, Da>(&args.rollup_config_path, args.db_path.as_deref())?;
        run_with_options::<S, H>(
            crate::MigrationOptions {
                storage,
                dry_run: args.dry_run,
            },
            accounts,
            chain_state,
        )
    }

    /// Runs the v1 migration, writes the report if requested, and returns the JSON report.
    pub fn run_and_write_report<S, H, Da>(
        args: crate::MigrationArgs,
        accounts: &mut Accounts<S>,
        chain_state: &mut ChainState<S>,
    ) -> anyhow::Result<String>
    where
        S: Spec,
        H: sov_rollup_interface::reexports::digest::Digest<
                OutputSize = sov_rollup_interface::reexports::digest::typenum::U32,
            > + Send
            + Sync,
        Da: sov_rollup_interface::node::da::DaService<Spec = S::Da>,
        sov_stf_runner::RollupConfig<S::Address, Da>: serde::de::DeserializeOwned,
        S::Storage: sov_db::storage_manager::InitializableNativeNomtStorage<
                H,
                <S::Da as sov_rollup_interface::da::DaSpec>::SlotHash,
            > + crate::MigrationStorage,
    {
        let report_out = args.report_out.clone();
        let report = run::<S, H, Da>(args, accounts, chain_state)?;
        crate::write_report(&report, report_out.as_deref())
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
            sov_modules_api::StateCheckpoint::new(storage, &kernel, None)
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
                .commit_migration_change_set_at_head(
                    head_slot_number,
                    change_set,
                    ledger_change_set,
                )
                .context("failed to commit migration changeset at head version")?;

            let (post_storage, post_ledger_reader) =
                storage_manager
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
                    "post-migration head slot changed unexpectedly: expected {}, found {}",
                    head_slot_number,
                    post_head_slot_number
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
}

/// Inputs for a migration that rewrites the current head state.
pub struct MigrationOptions {
    /// Database configuration for the rollup state to migrate.
    pub storage: RollupDbConfig,
    /// Compute the post-migration state root but do not commit changes.
    pub dry_run: bool,
}

/// JSON-serializable summary for an offline migration.
#[derive(Debug, Serialize)]
pub struct MigrationOutcome<Report> {
    /// Whether the migration was run without committing changes.
    pub dry_run: bool,
    /// Database path used by the migration.
    pub db_path: String,
    /// State root at the ledger head before the migration.
    pub pre_state_root: String,
    /// State root after applying the migration.
    pub post_state_root: String,
    /// Ledger head slot that was migrated.
    pub head_rollup_slot: u64,
    /// Migration-specific report.
    pub migration: Report,
}

/// Loads the storage config from a rollup TOML config and applies an optional DB path override.
pub fn load_storage_config<S, Da>(
    rollup_config_path: impl AsRef<Path>,
    db_path_override: Option<&Path>,
) -> anyhow::Result<RollupDbConfig>
where
    S: Spec,
    Da: DaService<Spec = S::Da>,
    RollupConfig<S::Address, Da>: serde::de::DeserializeOwned,
{
    let rollup_config: RollupConfig<S::Address, Da> = from_toml_path(&rollup_config_path)
        .with_context(|| {
            format!(
                "failed to read rollup config from {}",
                rollup_config_path.as_ref().display()
            )
        })?;

    let mut storage = rollup_config.storage;
    if let Some(db_path_override) = db_path_override {
        storage.path = db_path_override.to_path_buf();
    }
    Ok(storage)
}

/// Serializes a migration report and optionally writes it to a file.
pub fn write_report<Report: Serialize>(
    report: &MigrationOutcome<Report>,
    report_out: Option<&Path>,
) -> anyhow::Result<String> {
    let json = serde_json::to_string_pretty(report)
        .context("failed to serialize migration report JSON")?;

    if let Some(path) = report_out {
        fs::write(path, &json)
            .with_context(|| format!("failed to write migration report to {}", path.display()))?;
    }

    Ok(json)
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
