#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod app_template;
mod batch;
mod tx_verifier;

pub use app_template::AppTemplate;
pub use batch::Batch;
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::{Context, DispatchCall, Genesis, Spec};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::stf::{SlotResult, StateTransitionFunction};
use sov_rollup_interface::zk::{ValidityCondition, Zkvm};
use sov_state::{StateCheckpoint, Storage, WorkingSet};
use tracing::info;
pub use tx_verifier::RawTx;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use zk_cycle_macros::cycle_tracker;

/// This trait has to be implemented by a runtime in order to be used in `AppTemplate`.
pub trait Runtime<C: Context, Cond: ValidityCondition>:
    DispatchCall<Context = C>
    + Genesis<Context = C>
    + TxHooks<Context = C>
    + SlotHooks<Cond, Context = C>
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

impl<C: Context, RT, Vm: Zkvm, Cond: ValidityCondition, B: BlobReaderTrait>
    AppTemplate<C, Cond, Vm, RT, B>
where
    RT: Runtime<C, Cond>,
{
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn begin_slot(
        &mut self,
        slot_data: &impl SlotData<Cond = Cond>,
        witness: <Self as StateTransitionFunction<Vm, B>>::Witness,
    ) {
        let state_checkpoint = StateCheckpoint::with_witness(self.current_storage.clone(), witness);

        let mut working_set = state_checkpoint.to_revertable();

        self.runtime.begin_slot_hook(slot_data, &mut working_set);

        self.checkpoint = Some(working_set.checkpoint());
    }

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn end_slot(&mut self) -> (jmt::RootHash, <<C as Spec>::Storage as Storage>::Witness) {
        let (cache_log, witness) = self.checkpoint.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");

        let mut working_set = WorkingSet::new(self.current_storage.clone());

        self.runtime.end_slot_hook(&mut working_set);

        (jmt::RootHash(root_hash), witness)
    }
}

impl<C: Context, RT, Vm: Zkvm, Cond: ValidityCondition, B: BlobReaderTrait>
    StateTransitionFunction<Vm, B> for AppTemplate<C, Cond, Vm, RT, B>
where
    RT: Runtime<C, Cond>,
{
    type StateRoot = jmt::RootHash;

    type InitialState = <RT as Genesis>::Config;

    type TxReceiptContents = TxEffect;

    type BatchReceiptContents = SequencerOutcome;

    type Witness = <<C as Spec>::Storage as Storage>::Witness;

    type Condition = Cond;

    fn init_chain(&mut self, params: Self::InitialState) -> jmt::RootHash {
        let mut working_set = StateCheckpoint::new(self.current_storage.clone()).to_revertable();

        self.runtime
            .genesis(&params, &mut working_set)
            .expect("module initialization must succeed");

        let (log, witness) = working_set.checkpoint().freeze();
        let genesis_hash = self
            .current_storage
            .validate_and_commit(log, &witness)
            .expect("Storage update must succeed");

        jmt::RootHash(genesis_hash)
    }

    fn apply_slot<'a, I, Data>(
        &mut self,
        witness: Self::Witness,
        slot_data: &Data,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut B>,
        Data: SlotData<Cond = Self::Condition>,
    {
        self.begin_slot(slot_data, witness);

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
            batch_receipts.push(batch_receipt.clone());
        }

        let (state_root, witness) = self.end_slot();
        SlotResult {
            state_root,
            batch_receipts,
            witness,
        }
    }

    fn get_current_state_root(&self) -> anyhow::Result<Self::StateRoot> {
        self.current_storage
            .get_state_root(&Default::default())
            .map(jmt::RootHash)
    }
}
