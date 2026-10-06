//! Tests for the event frontier an [`ApiStateAccessor`] reports for the state it reads.
//!
//! The contract is that the reported frontier is *exact*: the snapshot reflects every event
//! below `next_event_number` in that epoch, and no event at or above it. These tests pin the
//! observable parts of that, including the epoch, which is what makes a reissued event number
//! distinguishable from a genuinely new one.

use std::sync::Arc;

use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{
    ApiStateAccessor, ConcurrentStateCheckpoint, EventEpoch, EventFrontier, StateCheckpoint,
    TxChangeSet,
};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::TestSpec;

use crate::state_tests::*;

/// Multi-byte, asymmetric values, so a byte-order mistake anywhere would show up.
const FIRST_NEXT_EVENT_NUMBER: u64 = 0x1234_5678;
const SECOND_NEXT_EVENT_NUMBER: u64 = 0x1234_56ab;

fn epoch(token: u128) -> EventEpoch {
    EventEpoch::new(token)
}

fn checkpoint(
    storage_manager: &mut SimpleStorageManager<StorageSpec>,
    kernel: &MockKernel<TestSpec>,
) -> ConcurrentStateCheckpoint<TestSpec> {
    let storage = storage_manager.create_prover_storage();
    ConcurrentStateCheckpoint::from_state_checkpoint(StateCheckpoint::new(storage, kernel))
}

fn accessor(
    checkpoint: Arc<ConcurrentStateCheckpoint<TestSpec>>,
    kernel: &MockKernel<TestSpec>,
) -> ApiStateAccessor<TestSpec> {
    ApiStateAccessor::new(checkpoint, Arc::new(kernel.clone()))
}

fn empty_changeset() -> TxChangeSet {
    TxChangeSet {
        writes: vec![],
        reads: Default::default(),
    }
}

#[test]
fn accessor_reports_the_checkpoints_frontier() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );

    assert_eq!(
        accessor(checkpoint, &kernel).event_frontier(),
        Some(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
        "The accessor should report the frontier of the checkpoint it reads"
    );
}

#[test]
fn frontier_reports_the_last_event_below_it() {
    let frontier = EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER);

    assert_eq!(
        frontier.last_event_number(),
        Some(FIRST_NEXT_EVENT_NUMBER - 1),
        "The last event reflected is one below the next number to be handed out"
    );
}

#[test]
fn frontier_reports_no_last_event_before_the_first_one() {
    let frontier = EventFrontier::new(epoch(0xabcd), 0);

    assert_eq!(
        frontier.last_event_number(),
        None,
        "A rollup that has not emitted any event has no last event number to report"
    );
}

#[test]
fn accessor_reports_no_frontier_when_the_producer_tracks_none() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(checkpoint(&mut storage_manager, &kernel));

    assert_eq!(
        accessor(checkpoint, &kernel).event_frontier(),
        None,
        "A checkpoint whose producer does not track event numbers must not report a frontier"
    );
}

#[test]
fn applying_a_transaction_advances_the_frontier_within_its_epoch() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );

    checkpoint.apply_tx_changes(empty_changeset(), Some(SECOND_NEXT_EVENT_NUMBER));

    assert_eq!(
        accessor(checkpoint, &kernel).event_frontier(),
        Some(EventFrontier::new(epoch(0xabcd), SECOND_NEXT_EVENT_NUMBER)),
        "Applying a transaction advances the number but stays in the same epoch: the numbering \
         was extended, not reissued"
    );
}

#[test]
fn a_transaction_without_events_leaves_the_frontier_alone() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );

    checkpoint.apply_tx_changes(empty_changeset(), None);

    assert_eq!(
        accessor(checkpoint, &kernel).event_frontier(),
        Some(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
        "A transaction that emitted no events attributes no new events to the state"
    );
}

#[test]
fn an_open_accessor_keeps_the_frontier_of_the_snapshot_it_opened() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );

    // Open a reader, then let a transaction land underneath it. This is the race the pairing
    // exists to rule out: the accessor's state snapshot does not include the new transaction,
    // so it must not report the new transaction's events either.
    let opened_before = accessor(checkpoint.clone(), &kernel);

    checkpoint.apply_tx_changes(empty_changeset(), Some(SECOND_NEXT_EVENT_NUMBER));

    assert_eq!(
        opened_before.event_frontier(),
        Some(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
        "An accessor must keep reporting the frontier paired with the snapshot it opened, not \
         one for state it cannot see"
    );
}

#[test]
fn a_reissued_number_is_distinguishable_by_its_epoch() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    // A rollback retracts events and then re-emits past where it started, so the number alone
    // only ever moves forward. The epoch is the only thing that reveals the reissue.
    let before = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );
    let after = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0x1234), SECOND_NEXT_EVENT_NUMBER)),
    );

    let before = accessor(before, &kernel).event_frontier().unwrap();
    let after = accessor(after, &kernel).event_frontier().unwrap();

    assert!(
        after.next_event_number > before.next_event_number,
        "This is the case the number cannot catch: it went up across the rollback"
    );
    assert_ne!(
        after.epoch, before.epoch,
        "The epoch must differ, otherwise a client would apply events {}.. onto a state built \
         from events that no longer exist",
        before.next_event_number
    );
}

#[test]
fn archival_accessors_report_no_frontier() {
    let mut storage_manager = SimpleStorageManager::new();
    let kernel = MockKernel::<TestSpec>::default();

    let checkpoint = Arc::new(
        checkpoint(&mut storage_manager, &kernel)
            .with_event_frontier(EventFrontier::new(epoch(0xabcd), FIRST_NEXT_EVENT_NUMBER)),
    );

    let archival = accessor(checkpoint, &kernel)
        .get_archival_state(sov_modules_api::capabilities::RollupHeight::new(0))
        .unwrap();

    assert_eq!(
        archival.event_frontier(),
        None,
        "The frontier describes the live checkpoint, so it says nothing about a historical read"
    );
}
