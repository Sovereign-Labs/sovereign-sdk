//! End-to-end tests that a malformed blob slashes a *registered* sequencer but is harmlessly
//! ignored when it comes from an *unregistered* DA address. This exercises the slashing wiring
//! around `deserialize_or_try_slash_sender` through the full STF.

use sov_mock_da::{MockAddress, MockBlob};
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;

use crate::helpers_basic_kernel::setup_basic_kernel;
use crate::{TestData, S};

fn is_registered(
    runner: &crate::TestRunner<crate::helpers_basic_kernel::BasicRT>,
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
