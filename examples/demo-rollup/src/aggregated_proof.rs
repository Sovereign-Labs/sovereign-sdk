//! Helper for reading the latest aggregated proof persisted in the ledger DB.

use sov_db::ledger_db::LedgerDb;
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;

/// Reads the raw bytes of the latest aggregated proof persisted in the ledger
/// DB, without verifying.
///
/// Panics if the DB read fails.
pub async fn read_latest_aggregated_proof(
    ledger_db: &LedgerDb,
) -> Option<SerializedAggregatedProof> {
    Some(
        ledger_db
            .get_latest_aggregated_proof()
            .await
            .expect("Failed to read latest aggregated proof from ledger DB")?
            .proof,
    )
}
