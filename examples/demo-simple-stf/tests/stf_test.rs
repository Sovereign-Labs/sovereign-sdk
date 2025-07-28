use demo_simple_stf::{ApplySlotResult, CheckHashPreimageStf, Root};
use sov_mock_da::verifier::MockDaSpec;
use sov_mock_da::{MockAddress, MockBlob, MockBlockHeader};
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::da::RelevantBlobIters;
use sov_rollup_interface::stf::{ExecutionContext, StateTransitionFunction};

#[test]
fn test_stf_success() {
    let address = MockAddress::from([1; 32]);

    let stf: &mut CheckHashPreimageStf = &mut CheckHashPreimageStf;
    StateTransitionFunction::<MockZkvm, MockZkvm, MockDaSpec>::init_chain(
        stf,
        &MockBlockHeader::default(),
        (),
        (),
    );

    let mut batch_blobs = {
        let incorrect_preimage = vec![1; 32];
        let correct_preimage = vec![0; 32];

        [
            MockBlob::new(incorrect_preimage, address, [0; 32]),
            MockBlob::new(correct_preimage, address, [0; 32]),
        ]
    };

    // Pretend we are in native code and progress the blobs to the verified state.
    for blob in &mut batch_blobs {
        blob.advance();
    }

    let mut proof_blobs = {
        [
            MockBlob::new(vec![0; 32], address, [0; 32]),
            MockBlob::new(vec![0; 32], address, [0; 32]),
        ]
    };

    for blob in &mut proof_blobs {
        blob.advance();
    }

    let relevant_blobs = RelevantBlobIters {
        proof_blobs: proof_blobs.as_mut_slice(),
        batch_blobs: batch_blobs.as_mut_slice(),
    };

    let result = StateTransitionFunction::<MockZkvm, MockZkvm, MockDaSpec>::apply_slot(
        stf,
        &Root([]),
        (),
        (),
        &MockBlockHeader::default(),
        relevant_blobs,
        ExecutionContext::Node,
    );

    assert_eq!(2, result.batch_receipts.len());

    let receipt = &result.batch_receipts[0];
    assert_eq!(receipt.inner, ApplySlotResult::Failure);

    let receipt = &result.batch_receipts[1];
    assert_eq!(receipt.inner, ApplySlotResult::Success);
}
