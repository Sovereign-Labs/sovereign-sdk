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

#[async_trait]
impl<S, Rt> ReplicaEventHandler for Arc<SequencerStateUpdator<S, Rt>>
where
    S: Spec,
    Rt: Runtime<S>,
{
    #[allow(clippy::match_same_arms)]
    async fn on_db_event(&self, data: DbData) -> Result<(), DBDataRejected> {
        let res: Result<(), ReplicaError<S>> = match data {
            DbData::BatchStart(_) => Ok(()),
            DbData::Transaction(_, _, _) => Ok(()),
            DbData::BatchEnd(_) => Ok(()),
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
            Err(ReplicaError::Shutdown) => return Ok(()),
            Err(ReplicaError::UnexpectedShutdown) => {
                panic!("Replica unexpectedly shut down");
            }
        };
    }
}
