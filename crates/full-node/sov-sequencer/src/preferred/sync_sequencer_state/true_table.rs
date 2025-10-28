use crate::preferred::sync_sequencer_state::InitialConditions;
use crate::preferred::sync_sequencer_state::TrueTable;
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
    table: TrueTable,
    info: &StateUpdateInfo<S::Storage>,
    inner: &mut InnerGuard<'_, S, Rt>,
    initial_conditions: InitialConditions,
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
            // We only need to replay the transactions in the edge cases where the event/tx cache needs repopulating.
            // In all other cases, we can just accept the new storage and move on.
            let executor = if initial_conditions.should_flush_tx_cache() {
                debug!(
                    ?initial_conditions,
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
                let rollup_height =
                    StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel())
                        .rollup_height_to_access();
                debug!(
                    ? initial_conditions,
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
    };

    operation
}

pub(crate) async fn operation_for_replica<S: Spec, Rt: Runtime<S>>(
    table: TrueTable,
    info: &StateUpdateInfo<S::Storage>,
    inner: &mut InnerGuard<'_, S, Rt>,
    initial_conditions: InitialConditions,
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
        (true, _, _, true, _) => {
            inner.executor_events_sender.clean_all_batches_from_cache();
            PreferredSeqOperation::WaitForNodeResyncToTip
        }
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
            // We only need to replay the transactions in the edge cases where the event/tx cache needs repopulating.
            // In all other cases, we can just accept the new storage and move on.
            let executor = if initial_conditions.should_flush_tx_cache() {
                debug!(
                    ?initial_conditions,
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
                let rollup_height =
                    StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel())
                        .rollup_height_to_access();
                debug!(
                    ? initial_conditions,
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
    };

    operation
}
