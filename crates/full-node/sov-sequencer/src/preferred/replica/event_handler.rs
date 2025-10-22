use crate::preferred::inner::ReplicaError;
use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::replica_sync_task::DBDataRejected;
use crate::preferred::replica::replica_sync_task::ReplicaEventHandler;
use crate::preferred::SequencerStateUpdatorError;
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
        let res = match data {
            DbData::BatchStart(batch_to_store) => {
                println!("Batch start {}", batch_to_store.sequence_number);

                self.do_batch_start_msg_replica(batch_to_store, "replica_start_batch")
                    .await
            }
            DbData::Transaction(_, tx, tx_hash) => {
                self.do_new_tx_msg_replica(tx_hash, tx, "replica_new_tx")
                    .await
            }
            DbData::BatchEnd(_batch_to_store) => {
                self.close_current_batch_msg_replica("replica_close_batch")
                    .await
            }
            DbData::NewProof => Ok(Ok(())),
        };

        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => match e {
                ReplicaError::Rejected(db_data_rejected) => return Err(db_data_rejected),
                ReplicaError::NotReady(sequencer_not_ready_details) => {
                    //DBDataRejected::ExecutorBehind(())
                    todo!()
                }
                ReplicaError::Creation(batch_creation_error) => panic!("TODO"),
                ReplicaError::NewTx(do_new_tx_error) => panic!("TODO"),
            },
            Err(e) => match e {
                SequencerStateUpdatorError::Shutdown => {
                    todo!()
                }
                SequencerStateUpdatorError::Unexpected => panic!("TODO"),
            },
        };

        Ok(())
    }
}
