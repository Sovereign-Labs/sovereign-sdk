use sov_rollup_interface::{
    da::BlobTransactionTrait,
    jmt::SimpleHasher,
    stf::{BatchReceipt, ConsensusSetUpdate, OpaqueAddress, StateTransitionFunction},
    zk::traits::Zkvm,
    Buf,
};
use std::io::Read;

#[derive(PartialEq, Debug, Clone, Eq, serde::Serialize, serde::Deserialize)]

pub struct CheckHashPreimageStf {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ApplyBlobResult {
    Failure,
    Success,
}

impl<VM: Zkvm> StateTransitionFunction<VM> for CheckHashPreimageStf {
    // Since our rollup is stateless, we don't need to consider the StateRoot.
    type StateRoot = ();

    // This represents the initial configuration of the rollup, but it is not supported in this tutorial.
    type InitialState = ();

    // We could incorporate the concept of a transaction into the rollup, but we leave it as an exercise for the reader.
    type TxReceiptContents = ();

    // This is the type that will be returned as a result of `apply_blob`.
    type BatchReceiptContents = ApplyBlobResult;

    // This data is produced during actual batch execution or validated with proof during verification.
    // However, in this tutorial, we won't use it.
    type Witness = ();

    // This represents a proof of misbehavior by the sequencer, but we won't utilize it in this tutorial.
    type MisbehaviorProof = ();

    // Perform one-time initialization for the genesis block.
    fn init_chain(&mut self, _params: Self::InitialState) {
        // Do nothing
    }

    // Called at the beginning of each DA-layer block - whether or not that block contains any
    // data relevant to the rollup.
    fn begin_slot(&mut self, _witness: Self::Witness) {
        // Do nothing
    }

    // The core logic of our rollup.
    fn apply_blob(
        &mut self,
        blob: impl BlobTransactionTrait,
        _misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> BatchReceipt<Self::BatchReceiptContents, Self::TxReceiptContents> {
        let blob_data = blob.data();
        let mut reader = blob_data.reader();

        // Read the data from the blob as a byte vec.
        let mut data = Vec::new();

        // Panicking within the `StateTransitionFunction` is generally not recommended.
        // But here if we encounter an error while reading the bytes, it suggests a serious issue with the DA layer or our setup.
        reader
            .read_to_end(&mut data)
            .unwrap_or_else(|e| panic!("Unable to read blob data {}", e));

        // Check if the sender submitted the preimage of the hash.
        let hash = sha2::Sha256::hash(&data);
        let desired_hash = [
            102, 104, 122, 173, 248, 98, 189, 119, 108, 143, 193, 139, 142, 159, 142, 32, 8, 151,
            20, 133, 110, 226, 51, 179, 144, 42, 89, 29, 13, 95, 41, 37,
        ];

        let result = if hash == desired_hash {
            ApplyBlobResult::Success
        } else {
            ApplyBlobResult::Failure
        };

        // Return the `BatchReceipt`
        BatchReceipt {
            batch_hash: hash,
            tx_receipts: vec![],
            inner: result,
        }

        // In the current implementation, every blob contains the data we pass to the hash function.
        // As an exercise for the reader, you can introduce the concept of transactions.
        // In this scenario, the blob would contain multiple transactions (containing data) that we can loop over to check hash equality.
        // The first transaction that finds the correct hash would break the loop and return early.
    }

    fn end_slot(
        &mut self,
    ) -> (
        Self::StateRoot,
        Self::Witness,
        Vec<ConsensusSetUpdate<OpaqueAddress>>,
    ) {
        ((), (), vec![])
    }
}
