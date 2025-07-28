use std::collections::HashMap;

use sov_mock_da::{MockBlob, MockDaSpec, MockHash};
use sov_modules_api::{BlobReaderTrait, DaSpec, Spec, VersionReader};
use sov_modules_stf_blueprint::{BatchReceipt, Runtime};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::runtime::{BlobStorage, SlotReceipt, ValueSetter};
use sov_test_utils::{EncodeCall, TestSequencer, TestSpec, TestUser};
mod helpers_basic_kernel;
mod helpers_soft_confirmations;

mod base_sequencing;
mod recovery_mode;
mod soft_confirmation;
mod unregistered_sequencer;

pub type S = sov_test_utils::TestSpec;
pub type Da = MockDaSpec;

pub type SlotConfigInfo<SequencerInfo> = Vec<SequencerInfo>;

pub struct TestData<S: Spec> {
    pub user: TestUser<S>,
    pub preferred_sequencer: TestSequencer<S>,
    pub regular_sequencer: TestSequencer<S>,
}

type TestRunner<RT> = sov_test_utils::runtime::TestRunner<RT, S>;

/// Returns the current visible rollup height in the runner.
pub fn visible_slot<RT: Runtime<S> + MinimalGenesis<S>>(runner: &TestRunner<RT>) -> u64 {
    runner
        .query_visible_state(|state| state.current_visible_slot_number())
        .get()
}

/// Returns the last `k` slot receipts
pub fn last_slot_receipts<RT: Runtime<S> + MinimalGenesis<S>>(
    runner: &TestRunner<RT>,
    k: usize,
) -> &[SlotReceipt<S>] {
    assert!(
        k <= runner.receipts().len(),
        "k must be less than or equal to the number of slots. k={}, number of slots={}",
        k,
        runner.receipts().len()
    );
    &runner.receipts()[runner.receipts().len() - k..]
}

/// Formats a batch receipt into a tuple of (batch_hash, sender) used for testing the blob storage.
fn format_batch_receipts(
    batch_receipts: &[BatchReceipt<S>],
) -> Vec<([u8; 32], <MockDaSpec as DaSpec>::Address)> {
    batch_receipts
        .iter()
        .map(|b| (b.batch_hash, b.inner.da_address))
        .collect::<Vec<_>>()
}

fn check_visible_slot_height(
    slot_num: usize,
    expected_visible_slot_heights_increases: Vec<u64>,
    current_visible_slot_height: u64,
    new_visible_slot_height: u64,
) {
    let expected_visible_slot_height = expected_visible_slot_heights_increases[slot_num];

    assert_eq!(
        expected_visible_slot_heights_increases[slot_num],
        new_visible_slot_height - current_visible_slot_height,
        "The visible slot height increase for the slot {slot_num} is not correct. Expected {expected_visible_slot_height}, but got new slot height {new_visible_slot_height}, current slot height {current_visible_slot_height}.",
    );
}

#[derive(Clone, Copy)]
struct SequenceInfo {
    id: usize,
    sequence_number: Option<usize>,
}

impl SequenceInfo {
    fn standard(id: usize) -> Self {
        Self {
            id,
            sequence_number: None,
        }
    }
}

/// This helper method asserts that given slots to send and an expected order of receipts, the
/// [`TestRunner`] will emit the receipts in the expected order. This helper method is
/// used in [`helpers_basic_kernel::assert_blobs_are_correctly_received_basic_kernel`] and [`helpers_soft_confirmations::assert_blobs_are_correctly_received_soft_confirmation`].
fn assert_blobs_are_correctly_received_helper<
    RT: Runtime<S> + MinimalGenesis<S> + EncodeCall<ValueSetter<S>>,
>(
    slots_to_send: Vec<RelevantBlobs<MockBlob>>,
    receive_order: Vec<Vec<SequenceInfo>>,
    expected_visible_slot_heights_increases: Vec<u64>,
    runner: &mut TestRunner<RT>,
) {
    assert_eq!(receive_order.len(), expected_visible_slot_heights_increases.len() , "The number of slots to receive and the number of expected visible slot heights don't match.");

    let mut current_visible_slot_height = visible_slot(runner);

    for (slot_num, slot) in slots_to_send.clone().into_iter().enumerate() {
        runner.execute::<RelevantBlobs<MockBlob>>(slot);

        let new_visible_slot_height = visible_slot(runner);
        check_visible_slot_height(
            slot_num,
            expected_visible_slot_heights_increases.clone(),
            current_visible_slot_height,
            new_visible_slot_height,
        );
        current_visible_slot_height = new_visible_slot_height;
    }

    // If this inequality is verified, it means that we need to run a few empty slots because
    // we are waiting for the blobs to be deferred.
    if receive_order.len() > slots_to_send.len() {
        for slot_num in 0..(receive_order.len() - slots_to_send.len()) {
            runner.advance_slots(1);

            let new_visible_slot_height = visible_slot(runner);
            check_visible_slot_height(
                slot_num + slots_to_send.len(),
                expected_visible_slot_heights_increases.clone(),
                current_visible_slot_height,
                new_visible_slot_height,
            );
            current_visible_slot_height = new_visible_slot_height;
        }
    }

    assert!(runner.receipts().len() >= receive_order.len(), "The execution has not produced enough receipts! Expected at least {} receipts, but got {}.", 
        receive_order.len(), runner.receipts().len());

    // We get all the receipts we received during the execution.
    let received_slots = last_slot_receipts(runner, receive_order.len())
        .iter()
        .map(|s| format_batch_receipts(s.batch_receipts()))
        .collect::<Vec<_>>();

    // We get all the blobs we sent during the execution. We flatten the map to have the list of blobs independently of the slots.
    let sent_slots = slots_to_send
        .iter()
        .flat_map(|blobs| {
            blobs
                .batch_blobs
                .iter()
                .map(|blob| (blob.hash(), blob.sender()))
        })
        .collect::<Vec<_>>();

    // We check that the blobs we received are the ones we sent in the correct order.
    for (received_slot_num, sent_blob_nums) in receive_order.iter().enumerate() {
        assert_eq!(
            sent_blob_nums.len(),
            received_slots[received_slot_num].len(),

            "We have not received the expected number of blobs for the slot {}. Expected {}, but got {}.",
            received_slot_num,
            sent_blob_nums.len(),
            received_slots[received_slot_num].len()
        );

        for (received_batch_num, sent_blob_num) in sent_blob_nums.iter().enumerate() {
            assert_eq!(
                sent_slots[sent_blob_num.id].0,
                MockHash(received_slots[received_slot_num][received_batch_num].0),
                "The blob hash for the blob number {received_batch_num} in the slot {received_slot_num} is not correct.",
            );

            assert_eq!(
                sent_slots[sent_blob_num.id].1,
                received_slots[received_slot_num][received_batch_num].1,
                "The blob sender for the blob number {received_batch_num} in the slot {received_slot_num} is not correct.",
            );

            if let Some(sequence_number) = sent_blob_num.sequence_number {
                runner.query_state(|state| {
                    assert!(BlobStorage::<TestSpec>::default()
                        .get_deferred_preferred_sequencer_blob(sequence_number as u64, state)
                        .unwrap()
                        .is_none());
                });
            }
        }
    }
}
