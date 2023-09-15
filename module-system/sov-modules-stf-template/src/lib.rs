#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod app_template;
mod batch;
mod tx_verifier;

pub use app_template::AppTemplate;
pub use batch::Batch;
use sov_modules_api::capabilities::BlobSelector;
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::{
    BasicAddress, BlobReaderTrait, Context, DaSpec, DispatchCall, Genesis, Spec, Zkvm,
};
use sov_rollup_interface::stf::{SlotResult, StateTransitionFunction};
use sov_state::{StateCheckpoint, Storage};
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;
use tracing::info;
pub use tx_verifier::RawTx;

/// This trait has to be implemented by a runtime in order to be used in `AppTemplate`.
pub trait Runtime<C: Context, Da: DaSpec>:
    DispatchCall<Context = C>
    + Genesis<Context = C>
    + TxHooks<Context = C>
    + SlotHooks<Da, Context = C>
    + ApplyBlobHooks<
        Da::BlobTransaction,
        Context = C,
        BlobResult = SequencerOutcome<
            <<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address,
        >,
    > + BlobSelector<Da, Context = C>
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
/// Represents the different outcomes that can occur for a sequencer after batch processing.
pub enum SequencerOutcome<A: BasicAddress> {
    /// Sequencer receives reward amount in defined token and can withdraw its deposit
    Rewarded(u64),
    /// Sequencer loses its deposit and receives no reward
    Slashed {
        /// Reason why sequencer was slashed.
        reason: SlashingReason,
        #[serde(bound(deserialize = ""))]
        /// Sequencer address on DA.
        sequencer_da_address: A,
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

impl<C, RT, Vm, Da> AppTemplate<C, Da, Vm, RT>
where
    C: Context,
    Vm: Zkvm,
    Da: DaSpec,
    RT: Runtime<C, Da>,
{
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn begin_slot(
        &mut self,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        witness: <Self as StateTransitionFunction<Vm, Da>>::Witness,
    ) {
        let state_checkpoint = StateCheckpoint::with_witness(self.current_storage.clone(), witness);
        let mut working_set = state_checkpoint.to_revertable();

        self.runtime
            .begin_slot_hook(slot_header, validity_condition, &mut working_set);

        self.checkpoint = Some(working_set.checkpoint());
    }

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn end_slot(&mut self) -> (jmt::RootHash, <<C as Spec>::Storage as Storage>::Witness) {
        let checkpoint = self.checkpoint.take().unwrap();

        // Run end end_slot_hook
        let mut working_set = checkpoint.to_revertable();
        self.runtime.end_slot_hook(&mut working_set);
        // Save checkpoint
        let mut checkpoint = working_set.checkpoint();

        let (cache_log, witness) = checkpoint.freeze();

        let (root_hash, authenticated_node_batch) = self
            .current_storage
            .compute_state_update(cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");

        let mut working_set = checkpoint.to_revertable();
        self.runtime
            .finalize_slot_hook([0; 32], &mut working_set.accessory_state());

        let accessory_log = working_set.checkpoint().freeze_non_provable();

        self.current_storage
            .commit(&authenticated_node_batch, &accessory_log);

        (jmt::RootHash(root_hash), witness)
    }
}

impl<C, RT, Vm, Da> StateTransitionFunction<Vm, Da> for AppTemplate<C, Da, Vm, RT>
where
    C: Context,
    Da: DaSpec,
    Vm: Zkvm,
    RT: Runtime<C, Da>,
{
    type StateRoot = jmt::RootHash;

    type InitialState = <RT as Genesis>::Config;

    type TxReceiptContents = TxEffect;

    type BatchReceiptContents = SequencerOutcome<<Da::BlobTransaction as BlobReaderTrait>::Address>;

    type Witness = <<C as Spec>::Storage as Storage>::Witness;

    type Condition = Da::ValidityCondition;

    fn init_chain(&mut self, params: Self::InitialState) -> jmt::RootHash {
        let mut working_set = StateCheckpoint::new(self.current_storage.clone()).to_revertable();

        self.runtime
            .genesis(&params, &mut working_set)
            .expect("module initialization must succeed");

        let mut checkpoint = working_set.checkpoint();
        let (log, witness) = checkpoint.freeze();
        let accessory_log = checkpoint.freeze_non_provable();

        let (genesis_hash, node_batch) = self
            .current_storage
            .compute_state_update(log, &witness)
            .expect("Storage update must succeed");

        self.current_storage.commit(&node_batch, &accessory_log);
        jmt::RootHash(genesis_hash)
    }

    fn apply_slot<'a, I>(
        &mut self,
        witness: Self::Witness,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        self.begin_slot(slot_header, validity_condition, witness);

        // Initialize batch workspace
        let mut batch_workspace = self
            .checkpoint
            .take()
            .expect("Working_set was initialized in begin_slot")
            .to_revertable();
        let selected_blobs = self
            .runtime
            .get_blobs_for_this_slot(blobs, &mut batch_workspace)
            .expect("blob selection must succeed, probably serialization failed");

        info!(
            "Selected {} blob(s) for execution in current slot",
            selected_blobs.len()
        );

        self.checkpoint = Some(batch_workspace.checkpoint());

        let mut batch_receipts = vec![];
        for (blob_idx, mut blob) in selected_blobs.into_iter().enumerate() {
            let batch_receipt = self
                .apply_blob(blob.as_mut_ref())
                .unwrap_or_else(Into::into);
            info!(
                "blob #{} from sequencer {} with blob_hash 0x{} has been applied with #{} transactions, sequencer outcome {:?}",
                blob_idx,
                blob.as_mut_ref().sender(),
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
