#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::fmt::Display;

use sha2::Digest;
use sov_rollup_interface::common::RollupHeight;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec, RelevantBlobIters};
use sov_rollup_interface::stf::{ApplySlotOutput, BatchReceipt, StateTransitionFunction};
use sov_rollup_interface::zk::Zkvm;

/// An implementation of the [`StateTransitionFunction`]
/// that is specifically designed to check if someone knows a preimage of a specific hash.
#[derive(PartialEq, Debug, Clone, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct CheckHashPreimageStf;

/// Outcome of the apply_slot method.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ApplySlotResult {
    /// Incorrect hash preimage was posted on the DA.
    Failure,
    /// Correct hash preimage was posted on the DA.
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// An empty state root
pub struct Root(pub [u8; 0]);

impl AsRef<[u8]> for Root {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Display for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Root")
    }
}

impl<InnerVm: Zkvm, OuterVm: Zkvm, Da: DaSpec> StateTransitionFunction<InnerVm, OuterVm, Da>
    for CheckHashPreimageStf
{
    // Since our rollup is stateless, we don't need to consider the StateRoot.
    type StateRoot = Root;

    type Address = [u8; 32];

    type GasPrice = ();

    // This represents the initial configuration of the rollup, but it is not supported in this tutorial.
    type GenesisParams = ();
    type PreState = ();
    type ChangeSet = ();

    // We could incorporate the concept of a transaction into the rollup, but we leave it as an exercise for the reader.
    type TxReceiptContents = ();

    // Similarly, we don't bother implementing special receipts for proofs in this tutorial
    type StorageProof = ();

    // This is the type that will be returned as a result of `apply_blob`.
    type BatchReceiptContents = ApplySlotResult;

    // This data is produced during actual batch execution or validated with proof during verification.
    // However, in this tutorial, we won't use it.
    type Witness = ();

    // Perform one-time initialization for the genesis block.
    fn init_chain(
        &self,
        _genesis_block_header: &Da::BlockHeader,

        _base_state: Self::PreState,
        _params: Self::GenesisParams,
    ) -> (Root, ()) {
        (Root([]), ())
    }

    fn apply_slot(
        &self,
        _pre_state_root: &Root,
        _base_state: Self::PreState,
        _witness: Self::Witness,
        _slot_header: &Da::BlockHeader,
        relevant_blobs: RelevantBlobIters<&mut [Da::BlobTransaction]>,
        _execution_context: sov_rollup_interface::stf::ExecutionContext,
    ) -> ApplySlotOutput<InnerVm, OuterVm, Da, Self> {
        let mut receipts = vec![];
        for blob in relevant_blobs.batch_blobs {
            let data = blob.verified_data();

            // Check if the sender submitted the preimage of the hash.
            let hash = sha2::Sha256::digest(data).into();
            let desired_hash = [
                102, 104, 122, 173, 248, 98, 189, 119, 108, 143, 193, 139, 142, 159, 142, 32, 8,
                151, 20, 133, 110, 226, 51, 179, 144, 42, 89, 29, 13, 95, 41, 37,
            ];

            let result = if hash == desired_hash {
                ApplySlotResult::Success
            } else {
                ApplySlotResult::Failure
            };

            // Return the `BatchReceipt`
            receipts.push(BatchReceipt {
                batch_hash: hash,
                tx_receipts: vec![],
                ignored_tx_receipts: vec![],
                inner: result,
            });
        }

        ApplySlotOutput::<InnerVm, OuterVm, Da, Self> {
            state_root: Root([]),
            change_set: (),
            proof_receipts: vec![],
            batch_receipts: receipts,
            discarded_blobs: Default::default(),
            witness: (),
            rollup_height: RollupHeight::new(0),
        }
    }
}
