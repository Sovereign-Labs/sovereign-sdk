use std::marker::PhantomData;

use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::stf::{BatchReceipt, SlotResult, StateTransitionFunction};
use sov_rollup_interface::zk::{ValidityCondition, Zkvm};

/// A mock implementation of the [`StateTransitionFunction`]
#[derive(PartialEq, Debug, Clone, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct MockStf<Cond> {
    phantom_data: PhantomData<Cond>,
}

impl<Vm: Zkvm, Cond: ValidityCondition, Da: DaSpec> StateTransitionFunction<Vm, Da>
    for MockStf<Cond>
{
    type StateRoot = [u8; 0];
    type GenesisParams = ();
    type PreState = ();
    type ChangeSet = ();
    type TxReceiptContents = ();
    type BatchReceiptContents = ();
    type Witness = ();
    type Condition = Cond;

    // Perform one-time initialization for the genesis block.
    fn init_chain(
        &self,
        _base_state: Self::PreState,
        _params: Self::GenesisParams,
    ) -> ([u8; 0], ()) {
        ([], ())
    }

    fn apply_slot<'a, I>(
        &self,
        _pre_state_root: &[u8; 0],
        _base_state: Self::PreState,
        _witness: Self::Witness,
        _slot_header: &Da::BlockHeader,
        _validity_condition: &Da::ValidityCondition,
        _blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::ChangeSet,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        SlotResult {
            state_root: [],
            change_set: (),
            batch_receipts: vec![BatchReceipt {
                batch_hash: [0; 32],
                tx_receipts: vec![],
                inner: (),
            }],
            witness: (),
        }
    }
}
