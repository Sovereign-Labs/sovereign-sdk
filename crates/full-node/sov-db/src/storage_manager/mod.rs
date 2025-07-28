//! Implementations of [`sov_rollup_interface::storage::HierarchicalStorageManager`].
mod delta_reader_based;
mod nomt_based;
#[cfg(test)]
pub mod tests;

pub use delta_reader_based::*;
pub use nomt_based::{
    InitializableNativeNomtStorage, NomtChangeSet, NomtStorageManager, StateFinishedSession,
};
use rockbound::cache::delta_reader::DeltaReader;

use crate::ledger_db::LedgerDb;

// Information about the latest finalized slot is outdated. We know that
// only finalized slots are ever persisted to the ledger db, so let's
// make sure queries about finalized slots reflect that.
// NOTE: This should be done only at the startup.
pub(crate) fn update_ledger_finalized_height(
    ledger: std::sync::Arc<rockbound::DB>,
) -> anyhow::Result<()> {
    let ledger_reader = DeltaReader::new(ledger.clone(), Vec::new());

    let ledger_db = LedgerDb::with_reader(ledger_reader.clone())?;

    if let Some((slot_num, _slot)) = ledger_db.get_head_slot()? {
        let ledger_changeset = ledger_db.materialize_latest_finalize_slot(slot_num, slot_num)?;
        ledger.write_schemas(&ledger_changeset)?;
    }

    Ok(())
}
