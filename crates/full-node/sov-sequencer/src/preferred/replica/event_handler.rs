use crate::preferred::replica::event_receiver::DbData;
use crate::preferred::replica::replica_sync_task::ReplicaEventHandler;
use async_trait::async_trait;

pub struct ReplicaEventProcessor {}

#[async_trait]
impl ReplicaEventHandler for ReplicaEventProcessor {
    async fn on_da_event(&self, _data: DbData) {}
}
