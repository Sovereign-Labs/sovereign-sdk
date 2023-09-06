use demo_simple_stf::{ApplySlotResult, CheckHashPreimageStf};
use sov_rollup_interface::mocks::{MockAddress, MockBlob, MockBlock, MockValidityCond, MockZkvm};
use sov_rollup_interface::stf::StateTransitionFunction;

#[test]
fn test_stf_success() {
    let address = MockAddress { addr: [1; 32] };

    let stf = &mut CheckHashPreimageStf::<MockValidityCond>::default();
    StateTransitionFunction::<MockZkvm, MockBlob>::init_chain(stf, ());

    let mut blobs = {
        let incorrect_preimage = vec![1; 32];
        let correct_preimage = vec![0; 32];

        [
            MockBlob::new(incorrect_preimage, address, [0; 32]),
            MockBlob::new(correct_preimage, address, [0; 32]),
        ]
    };

    let result = StateTransitionFunction::<MockZkvm, MockBlob>::apply_slot(
        stf,
        (),
        &MockBlock::default(),
        &mut blobs,
    );

    assert_eq!(2, result.batch_receipts.len());

    let receipt = &result.batch_receipts[0];
    assert_eq!(receipt.inner, ApplySlotResult::Failure);

    let receipt = &result.batch_receipts[1];
    assert_eq!(receipt.inner, ApplySlotResult::Success);
}
