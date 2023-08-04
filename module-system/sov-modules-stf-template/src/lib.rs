#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod app_template;
mod batch;
mod tx_verifier;

pub use app_template::AppTemplate;
pub use batch::Batch;
use sov_modules_api::hooks::{ApplyBlobHooks, TxHooks};
use sov_modules_api::{Context, DispatchCall, Genesis, Spec};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::stf::{SlotResult, StateTransitionFunction};
use sov_rollup_interface::zk::Zkvm;
use sov_state::{StateCheckpoint, Storage};
use tracing::info;
pub use tx_verifier::RawTx;

///TODO
pub trait Runtime<C: Context>:
    DispatchCall<Context = C>
    + Genesis<Context = C>
    + TxHooks<Context = C>
    + ApplyBlobHooks<Context = C, BlobResult = SequencerOutcome>
{
}

/// The receipts of all the transactions in a batch.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxEffect {
    /// Batch was reverted.
    Reverted,
    /// Batch was processed successfully.
    Successful,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// TODO: Should be generic for Address for pretty printing https://github.com/Sovereign-Labs/sovereign-sdk/issues/465
/// Represents the different outcomes that can occur for a sequencer after batch processing.
pub enum SequencerOutcome {
    /// Sequencer receives reward amount in defined token and can withdraw its deposit
    Rewarded(u64),
    /// Sequencer loses its deposit and receives no reward
    Slashed {
        /// Reason why sequencer was slashed.
        reason: SlashingReason,
        // Keep this comment for so it doesn't need to investigate serde issue again.
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/465
        // #[serde(bound(deserialize = ""))]
        /// Sequencer address on DA.
        sequencer_da_address: Vec<u8>,
    },
    /// Batch was ignored, sequencer deposit left untouched.
    Ignored,
}

/// Reason why sequencer was slashed.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlashingReason {
    /// This status indicates problem with batch deserialization.
    InvalidBatchEncoding,
    /// Stateless verification failed, for example deserialized transactions have invalid signatures.
    StatelessVerificationFailed,
    /// This status indicates problem with transaction deserialization.
    InvalidTransactionEncoding,
}

impl<C: Context, RT, Vm: Zkvm, B: BlobReaderTrait> AppTemplate<C, RT, Vm, B> {
    fn begin_slot(&mut self, witness: <<C as Spec>::Storage as Storage>::Witness) {
        self.checkpoint = Some(StateCheckpoint::with_witness(
            self.current_storage.clone(),
            witness,
        ));
    }

    fn end_slot(&mut self) -> (jmt::RootHash, <<C as Spec>::Storage as Storage>::Witness) {
        let (cache_log, witness) = self.checkpoint.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");
        (jmt::RootHash(root_hash), witness)
    }
}

impl<C: Context, RT, Vm: Zkvm, B: BlobReaderTrait> StateTransitionFunction<Vm, B>
    for AppTemplate<C, RT, Vm, B>
where
    RT: Runtime<C>,
{
    type StateRoot = jmt::RootHash;

    type InitialState = <RT as Genesis>::Config;

    type TxReceiptContents = TxEffect;

    type BatchReceiptContents = SequencerOutcome;

    type Witness = <<C as Spec>::Storage as Storage>::Witness;

    fn init_chain(&mut self, params: Self::InitialState) {
        let mut working_set = StateCheckpoint::new(self.current_storage.clone()).to_revertable();

        self.runtime
            .genesis(&params, &mut working_set)
            .expect("module initialization must succeed");

        let (log, witness) = working_set.checkpoint().freeze();
        self.current_storage
            .validate_and_commit(log, &witness)
            .expect("Storage update must succeed");
    }

    fn apply_slot<'a, I>(
        &mut self,
        witness: Self::Witness,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut B>,
    {
        self.begin_slot(witness);

        let mut batch_receipts = vec![];
        for (blob_idx, blob) in blobs.into_iter().enumerate() {
            let batch_receipt = self.apply_blob(blob).unwrap_or_else(Into::into);
            info!(
                "blob #{} with blob_hash 0x{} has been applied with #{} transactions, sequencer outcome {:?}",
                blob_idx,
                hex::encode(batch_receipt.batch_hash),
                batch_receipt.tx_receipts.len(),
                batch_receipt.inner
            );
            for (i, tx_receipt) in batch_receipt.tx_receipts.iter().enumerate() {
                info!(
                    "tx #{} hash: 0x{} result {:?}",
                    i,
                    hex::encode(tx_receipt.tx_hash),
                    tx_receipt.receipt
                );
            }
            batch_receipts.push(batch_receipt);
        }

        let (state_root, witness) = self.end_slot();

        SlotResult {
            state_root,
            batch_receipts,
            witness,
        }
    }
}
