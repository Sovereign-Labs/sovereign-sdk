use demo_simple_stf::{ApplySlotResult, CheckHashPreimageStf};
use sov_rollup_interface::mocks::{MockAddress, MockBlob, MockBlock, MockValidityCond, MockZkvm};
use sov_rollup_interface::stf::StateTransitionFunction;

#[test]
fn test_stf() {
    let address = MockAddress { addr: [1; 32] };
    let preimage = vec![0; 32];

    let test_blob = MockBlob::<MockAddress>::new(preimage, address, [0; 32]);
    let stf = &mut CheckHashPreimageStf::<MockValidityCond>::default();

    let data = MockBlock::default();
    let mut blobs = [test_blob];

    StateTransitionFunction::<MockZkvm, MockBlob<MockAddress>>::init_chain(stf, ());

    let result = StateTransitionFunction::<MockZkvm, MockBlob<MockAddress>>::apply_slot(
        stf,
        (),
        &data,
        &mut blobs,
    );

    assert_eq!(1, result.batch_receipts.len());
    let receipt = result.batch_receipts[0].clone();
    assert_eq!(receipt.inner, ApplySlotResult::Success);
}
