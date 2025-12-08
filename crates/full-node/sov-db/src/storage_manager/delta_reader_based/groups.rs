use std::sync::Arc;

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;

use crate::accessory_db::AccessoryDb;
use crate::ledger_db::LedgerDb;
use crate::state_db::StateDb;
use crate::storage_manager::{update_ledger_finalized_height, InitializableNativeStorage};

#[derive(Debug, Clone)]
pub(crate) struct SnapshotGroup {
    state: Arc<SchemaBatch>,
    accessory: Arc<SchemaBatch>,
    ledger: Arc<SchemaBatch>,
}

impl SnapshotGroup {
    pub(crate) fn new(
        state_change_set: SchemaBatch,
        accessory_change_set: SchemaBatch,
        ledger_change_set: SchemaBatch,
    ) -> Self {
        Self {
            state: Arc::new(state_change_set),
            accessory: Arc::new(accessory_change_set),
            ledger: Arc::new(ledger_change_set),
        }
    }
}

pub(crate) struct DbGroup {
    state: Arc<rockbound::DB>,
    accessory: Arc<rockbound::DB>,
    ledger: Arc<rockbound::DB>,
}

impl DbGroup {
    pub(crate) fn new_write(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let state_rocksdb = StateDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        let accessory_rocksdb =
            AccessoryDb::get_rockbound_options().default_setup_db_in_path(&path)?;
        let ledger_rocksdb = LedgerDb::get_rockbound_options().default_setup_db_in_path(&path)?;

        Ok(Self {
            state: Arc::new(state_rocksdb),
            accessory: Arc::new(accessory_rocksdb),
            ledger: Arc::new(ledger_rocksdb),
        })
    }

    // Even though rocksdb does not require &mut self, we use it here to indicate exclusive access.
    pub(crate) fn commit(&mut self, snapshot: SnapshotGroup) -> anyhow::Result<()> {
        let SnapshotGroup {
            state,
            accessory,
            ledger,
        } = snapshot;
        // State and accessory go first, as its data can be synced from DA.
        self.state.write_schemas(&state)?;
        self.accessory.write_schemas(&accessory)?;
        // Ledger goes last, as its data is used during the start.
        // So if ledger save failed, state and accessory will be synced from DA
        self.ledger.write_schemas(&ledger)?;
        Ok(())
    }

    pub(crate) fn create_storage<S: InitializableNativeStorage>(
        &self,
        rev_snapshots: Vec<SnapshotGroup>,
    ) -> anyhow::Result<(S, DeltaReader)> {
        let mut state_snapshots = Vec::with_capacity(rev_snapshots.len());
        let mut accessory_snapshots = Vec::with_capacity(rev_snapshots.len());
        let mut ledger_snapshots = Vec::with_capacity(rev_snapshots.len());

        for SnapshotGroup {
            state,
            accessory,
            ledger,
        } in rev_snapshots.into_iter().rev()
        {
            state_snapshots.push(state);
            accessory_snapshots.push(accessory);
            ledger_snapshots.push(ledger);
        }

        let state_reader = DeltaReader::new(self.state.clone(), state_snapshots);
        let state_db = StateDb::with_delta_reader(state_reader)?;
        let accessory_reader = DeltaReader::new(self.accessory.clone(), accessory_snapshots);
        let accessory_db = AccessoryDb::with_reader(accessory_reader)?;
        let ledger_reader = DeltaReader::new(self.ledger.clone(), ledger_snapshots);

        let storage = S::new(state_db, accessory_db);

        Ok((storage, ledger_reader))
    }

    pub(crate) fn update_ledger_finalized_height(&self) -> anyhow::Result<()> {
        update_ledger_finalized_height(self.ledger.clone())
    }
}
