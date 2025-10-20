use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::db_data::DbData;
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
    async fn on_da_event(&self, data: DbData) {
        match data {
            DbData::BatchStart(_batch_to_store) => {}
            DbData::Transaction(_tx) => {}
            DbData::BatchEnd(_batch_to_store) => {}
            DbData::NewProof => {}
        }
    }
}
