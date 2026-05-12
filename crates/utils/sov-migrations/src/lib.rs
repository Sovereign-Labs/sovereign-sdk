//! Reusable offline state migrations for Sovereign SDK rollups.

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

pub mod v1;

/// CLI-shaped inputs for running a migration against a rollup database.
#[derive(Debug, Clone)]
pub struct MigrationArgs {
    /// Path to the rollup config file used by the running node.
    pub rollup_config_path: PathBuf,
    /// Override `storage.path` from the rollup config.
    pub db_path: Option<PathBuf>,
    /// Compute the post-migration state root but do not commit changes.
    pub dry_run: bool,
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
