use crate::preferred::get_next_sequence_number_according_to_node;
use crate::preferred::sync_sequencer_state::ConditionsTable;
use crate::preferred::sync_sequencer_state::InitialStatus;
use crate::preferred::InnerGuard;
use crate::preferred::PreferredSeqOperation;
use crate::SequencerNotReadyDetails;
use sov_modules_api::macros::config_value;
use sov_modules_api::Runtime;
use sov_modules_api::Spec;
use sov_modules_api::StateCheckpoint;
use sov_modules_api::StateUpdateInfo;
use sov_modules_api::VersionReader;
use sov_rollup_interface::common::SlotNumber;
use tokio::time::Duration;
use tracing::debug;
use tracing::error;
use tracing::warn;

pub(crate) async fn operation_for_master<S: Spec, Rt: Runtime<S>>(
    table: ConditionsTable,
    info: &StateUpdateInfo<S::Storage>,
    inner: &mut InnerGuard<'_, S, Rt>,
    initial_status: InitialStatus,
    time_spent_fetching_batches: Duration,
    current_visible_slot_number: SlotNumber,
) -> PreferredSeqOperation<S, Rt> {
    let sync_status = &info.sync_status;
    let distance = sync_status.distance();

    let operation = match (
        table.condition_nodes_sequence_number_is_fresher,
        table.condition_too_close_to_deferred_slots_count_for_comfort,
        table.condition_node_is_lagging,
        table.condition_are_there_batches_to_replay,
        table.condition_node_is_unsynced_and_doesnt_know_it,
    ) {
        (true, _, _, true, _) => PreferredSeqOperation::Unreachable,
        (true, _, false, false, _) => {
            warn!("The node has a higher sequence number than the sequencer, but we're very close to the chain tip, i.e. we don't expect to be simply syncing. This could mean there is another preferred sequencer running (which is not supported and will likely lead to issues), or you very recently restarted the node and there's still some in-flight blobs. Resyncing to the chain tip.");
            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });
            PreferredSeqOperation::WaitForNodeResyncToTip
        }
        (_, _, true, _, _) => {
            warn!(?distance, "The sequencer must pause because the node has lagged behind the DA blockchain. This might lead to a brief downtime for users.");
            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });
            PreferredSeqOperation::WaitForNodeResyncWithAllowedSlack
        }
        (false, true, false, _, _) => {
            error!(
                    slot_number_according_to_node=%info.slot_number,
                    %current_visible_slot_number,
                    deferred_slots = %config_value!("DEFERRED_SLOTS_COUNT"),
                    "Sequencer has detected that it is past, or very close to, having the visible_slot_number lag behind the deferred_slots_count threshold. Normal operation will be suspended until this can be remedied.");
            inner.trigger_recovery(info).await;

            PreferredSeqOperation::RecoverAndCatchUp
        }
        // Node is out of sync and doesn't know it. This is a rare edge case after a DB wipe.
        (_, _, _, _, true) => {
            // Check for this condition after all of the normal "out-of-sync" conditions have been checked, because it may be possible for other unsynced conditions to trip this check
            // and we'd rather report the real root cause if there's a different one.
            warn!("The node is unsynced and doesn't know it. This probably means that you wiped the node DB and are resyncing.");
            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });
            PreferredSeqOperation::WaitForNodeResyncToTip
        }
        (false, false, false, _, _) => {
            reply_soft_confirmations(info, inner, initial_status, time_spent_fetching_batches).await
        }
    };

    operation
}

pub(crate) async fn operation_for_replica<S: Spec, Rt: Runtime<S>>(
    table: ConditionsTable,
    info: &StateUpdateInfo<S::Storage>,
    inner: &mut InnerGuard<'_, S, Rt>,
    initial_status: InitialStatus,
    time_spent_fetching_batches: Duration,
    current_visible_slot_number: SlotNumber,
) -> PreferredSeqOperation<S, Rt> {
    let sync_status = &info.sync_status;
    let distance = sync_status.distance();

    let node_next_sequence_number =
        get_next_sequence_number_according_to_node(info, &mut Rt::default());
    let next_internal_sequence_number = inner.sequence_number_of_next_blob;

    debug!(
        ?table,
        ?node_next_sequence_number,
        ?next_internal_sequence_number,
        ?sync_status,
        "operation_for_replica"
    );

    let operation = match (
        table.condition_nodes_sequence_number_is_fresher,
        table.condition_too_close_to_deferred_slots_count_for_comfort,
        table.condition_node_is_lagging,
        table.condition_are_there_batches_to_replay,
        table.condition_node_is_unsynced_and_doesnt_know_it,
    ) {
        (true, _, _, true, _) => {
            // The node is ahead of the replica sequencer, which still has batches to replay.
            // This can happen in the following scenario. The replica is applying soft-conf from the master via PG
            // but for some reasons the Batch landed on DA faster than was closed with PG notification.
            // In this case we will log an error as we want the PG notification to be much faster than the DA
            // and heal the sequencer by deleting the in flight batches from the cache.

            error!("The node has a higher sequence number than the sequencer, but it’s very close to the chain tip. 
            This indicates that replicas are receiving data through DA faster than through Postgres notifications. 
            node_next_sequence_number: {node_next_sequence_number}, next_internal_sequence_number: {next_internal_sequence_number}");

            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });

            inner
                .executor_events_sender
                .flush_transactions_cache(info.next_tx_number)
                .await;
            inner.executor_events_sender.clean_all_batches_from_cache();

            PreferredSeqOperation::WaitForNodeResyncToTip
        }
        (true, _, false, false, _) => {
            // The replica is near the chain tip and observes new batches on the DA.
            // This indicates that the master continues producing batches while the replica is still syncing.
            // We wait until the replica is no more than one block behind the tip and override the replica’s sequencer with the node’s state.
            // At this stage, the replica can start accepting PG notifications from the master.
            if sync_status.distance() <= 1 {
                inner.executor_events_sender.clean_all_batches_from_cache();
                inner
                    .executor_events_sender
                    .flush_transactions_cache(info.next_tx_number)
                    .await;

                let executor = Some(Box::new(
                    inner.new_executor_with_empty_uncommitted_changes(info),
                ));

                return PreferredSeqOperation::ReplaySoftConfirmationsOnTopOfNodeStateIfNecessary(
                    executor,
                    Duration::from_secs(0),
                );
            } else {
                inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                    target_da_height: sync_status.target_da_height(),
                    synced_da_height: sync_status.synced_da_height(),
                });

                PreferredSeqOperation::WaitForNodeResyncToTip
            }
        }
        (_, _, true, _, _) => {
            // The replica node is syncing.
            warn!(
                ?distance,
                "The sequencer must pause because the node has lagged behind the DA blockchain."
            );
            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });
            PreferredSeqOperation::WaitForNodeResyncToTip
        }
        (false, true, false, _, _) => {
            error!(
                    slot_number_according_to_node=%info.slot_number,
                    %current_visible_slot_number,
                    deferred_slots = %config_value!("DEFERRED_SLOTS_COUNT"),
                    "Sequencer has detected that it is past, or very close to, having the visible_slot_number lag behind the deferred_slots_count threshold.");
            panic!("Replica does not support automatic recovery.");
        }
        // Node is out of sync and doesn't know it. This is a rare edge case after a DB wipe.
        (_, _, _, _, true) => {
            // Check for this condition after all of the normal "out-of-sync" conditions have been checked, because it may be possible for other unsynced conditions to trip this check
            // and we'd rather report the real root cause if there's a different one.
            warn!("The node is unsynced and doesn't know it. This probably means that you wiped the node DB and are resyncing.");
            inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                target_da_height: sync_status.target_da_height(),
                synced_da_height: sync_status.synced_da_height(),
            });
            PreferredSeqOperation::WaitForNodeResyncToTip
        }
        (false, false, false, _, _) => {
            reply_soft_confirmations(info, inner, initial_status, time_spent_fetching_batches).await
        }
    };

    operation
}

async fn reply_soft_confirmations<S: Spec, Rt: Runtime<S>>(
    info: &StateUpdateInfo<S::Storage>,
    inner: &mut InnerGuard<'_, S, Rt>,
    initial_status: InitialStatus,
    time_spent_fetching_batches: Duration,
) -> PreferredSeqOperation<S, Rt> {
    // We only need to replay the transactions in the edge cases where the event/tx cache needs repopulating.
    // In all other cases, we can just accept the new storage and move on.
    let executor = if initial_status.should_flush_tx_cache() {
        debug!(
            ?initial_status,
            "Proceeding with `replay_soft_confirmations_on_top_of_node_state`"
        );
        inner
            .executor_events_sender
            .flush_transactions_cache(info.next_tx_number)
            .await;

        // On `should_flush_tx_cache` we have to refill the cache the first time we `replay_soft_confirmations_on_top_of_node_state`
        Some(Box::new(
            // Since we're replaying from the node state, don't reuse any uncommitted changes
            inner.new_executor_with_empty_uncommitted_changes(info),
        ))
    } else {
        let rollup_height = StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel())
            .rollup_height_to_access();
        debug!(
            ? initial_status,
            % rollup_height,
            ?info,
            "Skipping `replay_soft_confirmations_on_top_of_node_state`. Fast tracking info"
        );
        None
    };

    PreferredSeqOperation::ReplaySoftConfirmationsOnTopOfNodeStateIfNecessary(
        executor,
        time_spent_fetching_batches,
    )
}
