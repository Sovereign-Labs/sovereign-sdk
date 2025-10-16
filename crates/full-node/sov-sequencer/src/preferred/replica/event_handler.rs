use crate::preferred::inner::SequencerStateUpdator;
use crate::preferred::replica::event_receiver::DbData;
use crate::preferred::replica::replica_sync_task::ReplicaEventHandler;
use async_trait::async_trait;
use sov_modules_api::Runtime;
use sov_modules_api::Spec;

pub struct ReplicaEventProcessor {}

#[async_trait]
impl ReplicaEventHandler for ReplicaEventProcessor {
    async fn on_da_event(&self, _data: DbData) {}
}

#[async_trait]
impl<S, Rt> ReplicaEventHandler for SequencerStateUpdator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    async fn on_da_event(&self, data: DbData) {
        match data {
            DbData::BatchStart(_batch_to_store) => {
                //self.
            }
            DbData::Transaction(_tx) => {
                //let x1 = todo!();
                //let x2 = todo!();
                //let x3 = todo!();
                //self.accept_tx_msg(x1, x2, x3, "accept_tx_replica").await;
            }
            DbData::BatchEnd(_batch_to_store) => {
                //self.close_current_batch_msg("close_current_batch_replica")
                //    .await;
            }
            DbData::NewProof => {}
        }
    }
}

/*
async fn process_do_batch_start(
    &mut self,
    visible_slot_number_after_increase: VisibleSlotNumber,
    visible_slots_to_advance: NonZero<u8>,
    reason: &'static str,
) {
    let mut inner = self.get_inner_with_timing(reason).await;
    if let Err(err) = inner
        .inner_do_batch_start(visible_slot_number_after_increase, visible_slots_to_advance)
        .await
    {
        tracing::error!(
            error = %err,
            "Error: while calling inner_do_batch_start."
        );
        panic!("Error: while calling inner_do_batch_start. The sequencer can no longer accept transactions!");
    }
}*/

/*
async fn process_do_new_tx(
    &mut self,
    tx_hash: TxHash,
    baked_tx: FullyBakedTx,
    reason: &'static str,
) {
    let mut inner = self.get_inner_with_timing(reason).await;
    let execution_time_micros = inner.executor.replay_tx(tx_hash, baked_tx.clone()).await;
    inner
        .batch_size_tracker
        .add_tx(baked_tx.data.len(), execution_time_micros);
    inner
        .executor_events_sender
        .insert_tx_without_confirmation(baked_tx, tx_hash)
        .await;
    let checkpoint = inner
        .executor
        .checkpoint
        .clone_with_empty_witness_dropping_temp_cache();
    inner
        .executor_events_sender
        .force_update_api_state(checkpoint)
        .await;
}*/

/*
async fn inner_do_batch_start(
    &mut self,
    visible_slot_number_after_increase: VisibleSlotNumber,
    visible_slots_to_advance: NonZero<u8>,
) -> anyhow::Result<()> {
    if self.executor.has_in_progress_batch() {
        return Err(anyhow!(
            "Received open batch notification, but replica already has an open batch"
        ));
    }

    // Query the master's batch metadata to get the exact visible slot parameters used
    // let batch_metadata =
    //     query_batch_metadata_from_db(query_pool, sequence_number).await?;
    self.try_start_batch_with_parameters_from_master(
        visible_slot_number_after_increase,
        visible_slots_to_advance,
    )
    .await?;

    // Ensure the batch was successfully started
    if !self.executor.has_in_progress_batch() {
        panic!(
            "Replica: no batch in progress, and no batch could be started. This should not be possible under any circumstances as the master was able to create a batch at this point. Please report this bug. {:?} {:?}",
            &self.executor.checkpoint, self.latest_info
        );
    }

    Ok(())
}

  /// Creates and starts a batch for replicas using the exact visible slot parameters from the master
#[tracing::instrument(skip_all, level = "trace")]
async fn try_start_batch_with_parameters_from_master(
    &mut self,
    visible_slot_number_after_increase: VisibleSlotNumber,
    visible_slots_to_advance: NonZero<u8>,
) -> anyhow::Result<()> {
    if self.executor.has_in_progress_batch() {
        return Ok(());
    }

    // Calculate the correct visible_slots_to_advance for this replica based on its current state
    let current_visible_slot_number = self.executor.checkpoint.current_visible_slot_number();
    let replica_visible_slots_to_advance = visible_slot_number_after_increase.as_true()
        .checked_sub(current_visible_slot_number.as_true().get())
        .and_then(|diff| NonZero::new(diff.get().try_into().unwrap()))
        .ok_or_else(|| {
            error!(
                current_visible_slot_number = %current_visible_slot_number,
                target_visible_slot_number = %visible_slot_number_after_increase,
                "Cannot calculate visible slots to advance for replica: target is not greater than current"
            );
            anyhow!("Invalid visible slot number progression for replica".to_string())
        })?;

    assert_eq!(
        visible_slots_to_advance,
        replica_visible_slots_to_advance,
        "Sanity check failed: replica visible_slots_to_advance calculation different from master."
    );

    let node_state_root = self.node_root_hash()?;
    let sequence_number = self.get_and_inc_next_sequence_number();
    let min_profit_per_tx = self.seq_config.sequencer_kind_config.minimum_profit_per_tx;

    let start_block_data = StartBlockData {
        sanity_check_visible_slot_number_after_increase: visible_slot_number_after_increase,
        visible_increase: replica_visible_slots_to_advance,
        node_state_root: node_state_root.clone(),
        minimum_profit_per_tx: min_profit_per_tx,
    };

    self.executor.start_rollup_block(start_block_data).await;

    self.executor_events_sender
        .start_batch(
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            sequence_number,
            self.executor
                .checkpoint
                .clone_with_empty_witness_dropping_temp_cache(),
        )
        .await;

    Ok(())
}
*/

/*
pub(crate) async fn insert_tx_without_confirmation(
    &mut self,
    tx: FullyBakedTx,
    tx_hash: TxHash,
) {
    self.cache.insert_tx(tx, tx_hash).await;
}*/
