use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::event_receiver::DbData;
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
            DbData::BatchStart(batch_to_store) => {
                let _ = self
                    .do_batch_start_msg(
                        batch_to_store.visible_slot_number_after_increase,
                        batch_to_store.visible_slots_to_advance,
                        "foo",
                    )
                    .await;
            }
            DbData::Transaction(_tx) => {
                //let _ = self.do_new_tx_msg(tx, todo!(), "foo").await;
            }
            DbData::BatchEnd(_batch_to_store) => {
                let _ = self.close_current_batch_msg("foo").await;
            }
            DbData::NewProof => {}
        }
    }
}
