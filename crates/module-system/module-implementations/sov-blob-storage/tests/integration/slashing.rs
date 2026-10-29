//! End-to-end tests that a malformed blob slashes a *registered* sequencer but is harmlessly
//! ignored when it comes from an *unregistered* DA address. This exercises the slashing wiring
//! around `deserialize_or_try_slash_sender` through the full STF.
//!
//! Also checks that a sequencer slashed while one of its own valid blobs is already queued
//! (deferred in storage, or selected earlier in the same slot) does not make the STF panic when
//! that blob executes and its escrow is returned.

use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_stf_blueprint::Runtime;
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::SequencerInfo;

use crate::helpers_basic_kernel::setup_basic_kernel;
use crate::helpers_soft_confirmations::{
    build_soft_confirmation_blobs, setup_with_registration_soft_confirmation_kernel,
};
use crate::{visible_slot, TestData, TestRunner, S};

fn is_registered<RT: Runtime<S> + MinimalGenesis<S>>(
    runner: &TestRunner<RT>,
    da: &MockAddress,
) -> bool {
    runner.query_state(|state| {
        SequencerRegistry::<S>::default()
            .is_registered_sequencer(da, state)
            .unwrap()
    })
}

fn garbage_blob(sender: MockAddress) -> RelevantBlobs<MockBlob> {
    // Random bytes can never deserialize as a preferred batch/proof.
    RelevantBlobs {
        proof_blobs: Vec::new(),
        batch_blobs: vec![MockBlob::new_with_hash(vec![0xff; 100], sender)],
    }
}

#[test]
fn malformed_blob_from_registered_sequencer_slashes_it() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();
    let da = preferred_sequencer.da_address;

    assert!(
        is_registered(&runner, &da),
        "the preferred sequencer must be registered at genesis"
    );

    runner.execute::<RelevantBlobs<MockBlob>>(garbage_blob(da));

    assert!(
        !is_registered(&runner, &da),
        "a registered sequencer must be slashed after submitting a malformed blob"
    );
}

#[test]
fn malformed_blob_from_unregistered_sender_is_ignored() {
    let (_, mut runner) = setup_basic_kernel();
    let da = MockAddress::new([99; 32]);

    assert!(
        !is_registered(&runner, &da),
        "the sender must not be registered to begin with"
    );

    // Must not panic, and must not register/slash an unknown sender (the `else` branch in
    // `deserialize_or_try_slash_sender`).
    runner.execute::<RelevantBlobs<MockBlob>>(garbage_blob(da));

    assert!(
        !is_registered(&runner, &da),
        "an unregistered sender must remain unregistered after a malformed blob"
    );
}

fn last_slot_batch_senders<RT: Runtime<S> + MinimalGenesis<S>>(
    runner: &TestRunner<RT>,
) -> Vec<MockAddress> {
    runner
        .receipts()
        .last()
        .unwrap()
        .batch_receipts
        .iter()
        .map(|b| b.inner.da_address)
        .collect()
}

/// - Slot A: a registered non-preferred sequencer posts a valid batch, which is deferred.
/// - Slot B: the same DA address posts garbage and gets slashed (registry entry deleted).
/// - Slot C: the preferred sequencer advances the visible slot over slot A, so the stored batch
///   executes after its sender was slashed.
#[test]
fn deferred_blob_of_slashed_sequencer_executes_without_panic() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();
    let da = regular_sequencer.da_address;
    let visible_before = visible_slot(&runner);
    let mut nonces = runner.nonces().clone();

    // Slot A: a non-empty batch, so that a non-zero amount is escrowed for it.
    let blobs = build_soft_confirmation_blobs(
        &vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        &mut nonces,
        1,
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    assert_eq!(
        visible_slot(&runner),
        visible_before,
        "The regular sequencer's batch must be deferred"
    );

    // Slot B
    runner.execute::<RelevantBlobs<MockBlob>>(garbage_blob(da));
    assert!(
        !is_registered(&runner, &da),
        "Garbage blob must slash the sender"
    );

    // Slot C
    let blobs = build_soft_confirmation_blobs(
        &vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 3,
                sequence_number: 1,
            },
        )],
        &mut nonces,
        0,
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);

    assert_eq!(
        last_slot_batch_senders(&runner),
        vec![preferred_sequencer.da_address, da],
        "The preferred batch and the slashed sequencer's stored batch must both execute"
    );
}

/// One DA block contains [preferred batch advancing visible, regular sequencer's valid empty batch,
/// regular sequencer's garbage]. The valid batch is selected before the garbage slashes its sender,
/// and executes later in the same slot.
#[test]
fn same_slot_valid_then_garbage_from_sequencer_executes_without_panic() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();
    let da = regular_sequencer.da_address;

    let mut nonces = runner.nonces().clone();
    let mut blobs = build_soft_confirmation_blobs(
        &vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 1,
                },
            ),
            (regular_sequencer.clone(), SequencerInfo::Regular),
        ],
        &mut nonces,
        0,
    );
    blobs.batch_blobs.extend(garbage_blob(da).batch_blobs);
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);

    assert!(
        !is_registered(&runner, &da),
        "Garbage blob must slash the sender"
    );
    assert_eq!(
        last_slot_batch_senders(&runner),
        vec![preferred_sequencer.da_address, da],
        "The preferred batch and the slashed sequencer's selected batch must both execute"
    );
}
