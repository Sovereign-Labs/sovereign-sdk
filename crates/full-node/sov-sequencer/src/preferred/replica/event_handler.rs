use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::replica_sync_task::DBDataRejected;
use crate::preferred::replica::replica_sync_task::ReplicaEventHandler;
use crate::preferred::BatchCreationError;
use crate::preferred::DoNewTxError;
use crate::preferred::SequencerStateUpdatorError;
use crate::SequencerNotReadyDetails;
use async_trait::async_trait;
use sov_modules_api::Runtime;
use sov_modules_api::Spec;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReplicaError<S: Spec> {
    #[error("The replica rejected the db data. DbData: {0:?}")]
    Rejected(DBDataRejected),

    #[error("Replica is not ready. Details: {0:?}. DbData: {1:?}")]
    NotReady(SequencerNotReadyDetails, DbData),

    #[error("Failed to create a new batch on the replica.")]
    Creation(#[from] BatchCreationError),

    #[error("Failed to apply a new transaction on the replica.")]
    NewTx(DoNewTxError<S>),

    #[error("The replica is shutting down.")]
    Shutdown,
    #[error("The replica encountered an unexpected shutdown.")]
    UnexpectedShutdown,
}

impl<S: Spec> From<SequencerStateUpdatorError> for ReplicaError<S> {
    fn from(value: SequencerStateUpdatorError) -> Self {
        match value {
            SequencerStateUpdatorError::Shutdown => Self::Shutdown,
            SequencerStateUpdatorError::Unexpected => Self::UnexpectedShutdown,
        }
    }
}

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
static GLOBAL_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    GLOBAL_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn get_counter() -> u64 {
    GLOBAL_COUNTER.load(Ordering::Relaxed)
}

#[async_trait]
impl<S, Rt> ReplicaEventHandler for Arc<SequencerStateUpdator<S, Rt>>
where
    S: Spec,
    Rt: Runtime<S>,
{
    #[allow(clippy::match_same_arms)]
    async fn on_db_event(&self, data: DbData) -> Result<(), DBDataRejected> {
        let res: Result<(), ReplicaError<S>> = match data {
            DbData::BatchStart(batch_to_store) => {
                println!("BatchStart {:?}", batch_to_store);
                self.do_batch_start_msg_replica(batch_to_store, "replica_start_batch")
                    .await
            }
            DbData::Transaction(seq, tx, tx_hash) => {
                let id = next_id();
                println!("Transaction {:?} {:?}", id, seq);
                self.do_new_tx_msg_replica(seq, tx_hash, tx, "replica_new_tx")
                    .await
            }
            DbData::BatchEnd(batch_to_store) => {
                println!("BatchEnd {:?}", batch_to_store);
                self.close_current_batch_msg_replica(batch_to_store, "replica_close_batch")
                    .await
            }
            DbData::NewProof => Ok(()),
        };

        match res {
            Ok(_) => return Ok(()),
            Err(ReplicaError::Rejected(db_data_rejected)) => return Err(db_data_rejected),
            Err(ReplicaError::NotReady(_sequencer_not_ready_details, db_data_rejected)) => {
                return Err(DBDataRejected::ExecutorBehind(db_data_rejected))
            }
            Err(ReplicaError::Creation(batch_creation_error)) => {
                panic!("Replica failed to create a new batch. Error: {batch_creation_error:?}");
            }
            Err(ReplicaError::NewTx(error)) => {
                panic!("Replica failed to apply a new transaction. Error: {error:?}");
            }
            Err(ReplicaError::Shutdown) => {
                // Replica shut down gracefully, this case is handled by the caller.
                return Ok(());
            }
            Err(ReplicaError::UnexpectedShutdown) => {
                panic!("Replica unexpectedly shut down");
            }
        };
    }
}
