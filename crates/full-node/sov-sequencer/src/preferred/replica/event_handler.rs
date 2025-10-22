use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::replica_sync_task::DBDataRejected;
use crate::preferred::replica::replica_sync_task::ReplicaEventHandler;
use async_trait::async_trait;
use sov_modules_api::Runtime;
use sov_modules_api::Spec;
use std::sync::Arc;

#[async_trait]
impl<S, Rt> ReplicaEventHandler for Arc<SequencerStateUpdator<S, Rt>>
where
    S: Spec,
    Rt: Runtime<S>,
{
    #[allow(clippy::match_same_arms)]
    async fn on_db_event(&self, data: DbData) -> Result<(), DBDataRejected> {
        match data {
            DbData::BatchStart(batch_to_store) => {
                println!("Batch start {}", batch_to_store.sequence_number);

                let _ = self
                    .do_batch_start_msg_replica(batch_to_store, "replica_start_batch")
                    .await
                    .unwrap();
            }
            DbData::Transaction(_, tx, tx_hash) => {
                self.do_new_tx_msg_replica(tx_hash, tx, "replica_new_tx")
                    .await
                    .unwrap();
            }
            DbData::BatchEnd(_batch_to_store) => {
                self.close_current_batch_msg("replica_close_batch")
                    .await
                    .unwrap();
            }
            DbData::NewProof => {}
        };

        Ok(())
    }
}
