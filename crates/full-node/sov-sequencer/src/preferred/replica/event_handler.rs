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
            DbData::BatchStart(_) => {}
            DbData::Transaction(_, _, _) => {}
            DbData::BatchEnd(_) => {}
            DbData::NewProof => {}
        };

        Ok(())
    }
}
