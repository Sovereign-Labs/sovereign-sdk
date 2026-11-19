//! Reusable offline state migrations for Sovereign SDK rollups.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use rockbound::SchemaBatch;
use serde::{Deserialize, Serialize};
use sov_chain_state::ChainState;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::tables::SlotByNumber;
use sov_db::storage_manager::NomtChangeSet;
use sov_full_node_configs::runner::from_toml_path;
use sov_modules_api::capabilities::{BlockGasInfo, RollupHeight};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::runtime::capabilities::Kernel as KernelTrait;
use sov_modules_api::{BootstrapWorkingSet, Spec, StateCheckpoint};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::NativeStorage;

pub mod v1;

#[derive(Deserialize)]
struct StorageOnlyRollupConfig {
    storage: RollupDbConfig,
}

/// CLI-shaped inputs for running a migration against a rollup database.
#[derive(Debug, Clone)]
pub struct MigrationArgs {
    /// Path to the rollup config file used by the running node.
    pub rollup_config_path: PathBuf,
    /// Overrides for the database locations in the rollup config.
    pub db_paths: DbPathOverrides,
    /// Compute the post-migration state root but do not commit changes.
    pub dry_run: bool,
}

/// Overrides for the database locations in the `storage` section of a rollup config.
///
/// Each field that is set replaces the matching `storage` field.
#[derive(Debug, Clone, Default)]
pub struct DbPathOverrides {
    /// Overrides `storage.path`.
    pub db_path: Option<PathBuf>,
    /// Overrides `storage.live_db_path`.
    pub live_db_path: Option<PathBuf>,
    /// Overrides `storage.ledger_db_path`.
    pub ledger_db_path: Option<PathBuf>,
    /// Overrides `storage.user_nomt_db_path`.
    pub user_nomt_db_path: Option<PathBuf>,
}

impl DbPathOverrides {
    /// Applies the overrides to `storage`.
    ///
    /// Fails if `db_path` is set while the config pins an exact database path that is not
    /// overridden too: that database would not move with `db_path`, so the migration would
    /// silently modify the database at the original location.
    pub fn apply(&self, storage: &mut RollupDbConfig) -> anyhow::Result<()> {
        if let Some(db_path) = &self.db_path {
            let pinned = [
                ("live_db_path", &storage.live_db_path, &self.live_db_path),
                (
                    "ledger_db_path",
                    &storage.ledger_db_path,
                    &self.ledger_db_path,
                ),
                (
                    "user_nomt_db_path",
                    &storage.user_nomt_db_path,
                    &self.user_nomt_db_path,
                ),
            ];
            for (field, configured, overridden) in pinned {
                if let (Some(configured), None) = (configured, overridden) {
                    bail!(
                        "rollup config sets `storage.{field}` = {}, which is not relocated by --db-path {}; \
                         pass --{} explicitly",
                        configured.display(),
                        db_path.display(),
                        field.replace('_', "-"),
                    );
                }
            }
            storage.path = db_path.clone();
        }
        if let Some(live_db_path) = &self.live_db_path {
            storage.live_db_path = Some(live_db_path.clone());
        }
        if let Some(ledger_db_path) = &self.ledger_db_path {
            storage.ledger_db_path = Some(ledger_db_path.clone());
        }
        if let Some(user_nomt_db_path) = &self.user_nomt_db_path {
            storage.user_nomt_db_path = Some(user_nomt_db_path.clone());
        }
        Ok(())
    }
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

/// Loads the storage config from a rollup TOML config and applies the DB path overrides.
pub fn load_storage_config(
    rollup_config_path: impl AsRef<Path>,
    overrides: &DbPathOverrides,
) -> anyhow::Result<RollupDbConfig> {
    let rollup_config: StorageOnlyRollupConfig =
        from_toml_path(&rollup_config_path).with_context(|| {
            format!(
                "failed to read rollup config from {}",
                rollup_config_path.as_ref().display()
            )
        })?;

    let mut storage = rollup_config.storage;
    overrides.apply(&mut storage)?;
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sov_db::config::RollupDbConfig;

    use super::DbPathOverrides;

    fn storage_in_data() -> RollupDbConfig {
        serde_json::from_str(r#"{"path":"data"}"#).unwrap()
    }

    #[test]
    fn per_db_overrides_replace_config_paths() {
        let mut storage = storage_in_data();
        storage.live_db_path = Some(PathBuf::from("/config/live"));
        storage.ledger_db_path = Some(PathBuf::from("/config/ledger"));
        storage.user_nomt_db_path = Some(PathBuf::from("/config/user"));

        DbPathOverrides {
            db_path: None,
            live_db_path: Some(PathBuf::from("/cli/live")),
            ledger_db_path: Some(PathBuf::from("/cli/ledger")),
            user_nomt_db_path: Some(PathBuf::from("/cli/user")),
        }
        .apply(&mut storage)
        .unwrap();

        assert_eq!(storage.path, PathBuf::from("data"));
        assert_eq!(storage.live_db_path, Some(PathBuf::from("/cli/live")));
        assert_eq!(storage.ledger_db_path, Some(PathBuf::from("/cli/ledger")));
        assert_eq!(storage.user_nomt_db_path, Some(PathBuf::from("/cli/user")));
    }

    #[test]
    fn db_path_replaces_path_when_config_has_no_per_db_paths() {
        let mut storage = storage_in_data();

        DbPathOverrides {
            db_path: Some(PathBuf::from("/cli/data")),
            ..Default::default()
        }
        .apply(&mut storage)
        .unwrap();

        assert_eq!(storage.path, PathBuf::from("/cli/data"));
        assert_eq!(storage.live_db_path, None);
        assert_eq!(storage.ledger_db_path, None);
        assert_eq!(storage.user_nomt_db_path, None);
    }

    #[test]
    fn db_path_rejected_when_config_pins_ledger_db_path() {
        let mut storage = storage_in_data();
        storage.ledger_db_path = Some(PathBuf::from("/config/ledger"));
        let original = storage.clone();

        let err = DbPathOverrides {
            db_path: Some(PathBuf::from("/cli/data")),
            ..Default::default()
        }
        .apply(&mut storage)
        .unwrap_err();

        assert!(
            err.to_string().contains("--ledger-db-path"),
            "error should name the flag that resolves the conflict, got: {err}"
        );
        assert_eq!(storage, original, "config must be left untouched on error");
    }

    #[test]
    fn db_path_accepted_when_config_per_db_paths_are_overridden() {
        let mut storage = storage_in_data();
        storage.live_db_path = Some(PathBuf::from("/config/live"));
        storage.ledger_db_path = Some(PathBuf::from("/config/ledger"));

        DbPathOverrides {
            db_path: Some(PathBuf::from("/cli/data")),
            live_db_path: Some(PathBuf::from("/cli/live")),
            ledger_db_path: Some(PathBuf::from("/cli/ledger")),
            user_nomt_db_path: None,
        }
        .apply(&mut storage)
        .unwrap();

        assert_eq!(storage.path, PathBuf::from("/cli/data"));
        assert_eq!(storage.live_db_path, Some(PathBuf::from("/cli/live")));
        assert_eq!(storage.ledger_db_path, Some(PathBuf::from("/cli/ledger")));
        assert_eq!(storage.user_nomt_db_path, None);
    }
}
