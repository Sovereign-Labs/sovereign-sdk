pub mod app_template;
mod batch;
mod tx_verifier;

pub use app_template::AppTemplate;
pub use batch::Batch;
use tracing::{debug, error, log::info};
pub use tx_verifier::RawTx;

use sov_modules_api::{
    hooks::{ApplyBlobHooks, SyncHooks, TxHooks},
    Context, DispatchCall, Genesis, Spec,
};
use sov_rollup_interface::stf::{BatchReceipt, SyncReceipt};
use sov_rollup_interface::stf::{StateTransitionFunction, TransactionReceipt};
use sov_rollup_interface::zk::traits::Zkvm;
use sov_state::StateCheckpoint;
use sov_state::Storage;
use std::io::Read;

use crate::{app_template::ApplyBatchError, tx_verifier::TransactionAndRawHash};

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxEffect {
    Reverted,
    Successful,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SequencerOutcome {
    /// Sequencer receives reward amount in defined token and can withdraw its deposit
    Rewarded(u64),
    /// Sequencer loses its deposit and receives no reward
    Slashed {
        reason: SlashingReason,
        sequencer_da_address: Vec<u8>,
    },
    /// Batch was ignored, sequencer deposit left untouched.
    Ignored,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlashingReason {
    InvalidBatchEncoding,
    StatelessVerificationFailed,
    InvalidTransactionEncoding,
}

impl<C: Context, RT, Vm: Zkvm> StateTransitionFunction<Vm> for AppTemplate<C, RT, Vm>
where
    RT: DispatchCall<Context = C>
        + Genesis<Context = C>
        + TxHooks<Context = C>
        + ApplyBlobHooks<Context = C, BlobResult = SequencerOutcome>
        + SyncHooks<Context = C>,
{
    type StateRoot = jmt::RootHash;

    type InitialState = <RT as Genesis>::Config;

    type TxReceiptContents = TxEffect;

    type BatchReceiptContents = SequencerOutcome;

    type Witness = <<C as Spec>::Storage as Storage>::Witness;

    type MisbehaviorProof = ();

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

    fn begin_slot(&mut self, witness: Self::Witness) {
        self.checkpoint = Some(StateCheckpoint::with_witness(
            self.current_storage.clone(),
            witness,
        ));
    }

    fn apply_tx_blob(
        &mut self,
        blob: &mut impl sov_rollup_interface::da::BlobTransactionTrait,
        _misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> BatchReceipt<Self::BatchReceiptContents, Self::TxReceiptContents> {
        debug!(
            "Applying batch from sequencer: 0x{}",
            hex::encode(blob.sender())
        );

        // Initialize batch workspace
        let mut batch_workspace = self
            .checkpoint
            .take()
            .expect("Working_set was initialized in begin_slot")
            .to_revertable();

        // ApplyBlobHook: begin
        if let Err(e) = self.runtime.begin_blob_hook(blob, &mut batch_workspace) {
            error!(
                "Error: The batch was rejected by the 'begin_blob_hook' hook. Skipping batch without slashing the sequencer: {}",
                e
            );
            // TODO: will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
            self.checkpoint = Some(batch_workspace.revert());
            return ApplyBatchError::Ignored(blob.hash()).into();
        }
        batch_workspace = batch_workspace.checkpoint().to_revertable();

        // TODO: don't ignore these events: https://github.com/Sovereign-Labs/sovereign/issues/350
        let _ = batch_workspace.take_events();

        let (txs, messages) = match self.pre_process_batch(blob.data_mut()) {
            Ok((txs, messages)) => (txs, messages),
            Err(reason) => {
                // Explicitly revert on slashing, even though nothing has changed in pre_process.
                let mut batch_workspace = batch_workspace.revert().to_revertable();
                let sequencer_da_address = blob.sender().as_ref().to_vec();
                let sequencer_outcome = SequencerOutcome::Slashed {
                    reason,
                    sequencer_da_address: sequencer_da_address.clone(),
                };
                match self
                    .runtime
                    .end_blob_hook(sequencer_outcome, &mut batch_workspace)
                {
                    Ok(()) => {
                        // TODO: will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
                        self.checkpoint = Some(batch_workspace.checkpoint());
                    }
                    Err(e) => {
                        error!("End blob hook failed: {}", e);
                        self.checkpoint = Some(batch_workspace.revert());
                    }
                };

                return (ApplyBatchError::Slashed {
                    hash: blob.hash(),
                    reason,
                    sequencer_da_address,
                })
                .into();
            }
        };

        // Sanity check after pre processing
        assert_eq!(
            txs.len(),
            messages.len(),
            "Error in preprocessing batch, there should be same number of txs and messages"
        );

        // Dispatching transactions
        let mut tx_receipts = Vec::with_capacity(txs.len());
        for (TransactionAndRawHash { tx, raw_tx_hash }, msg) in
            txs.into_iter().zip(messages.into_iter())
        {
            // Pre dispatch hook
            let sender_address = match self.runtime.pre_dispatch_tx_hook(&tx, &mut batch_workspace)
            {
                Ok(verified_tx) => verified_tx,
                Err(e) => {
                    // Don't revert any state changes made by the pre_dispatch_hook even if the Tx is rejected.
                    // For example nonce for the relevant account is incremented.
                    error!("Stateful verification error - the sequencer included an invalid transaction: {}", e);
                    let receipt = TransactionReceipt {
                        tx_hash: raw_tx_hash,
                        body_to_save: None,
                        events: batch_workspace.take_events(),
                        receipt: TxEffect::Reverted,
                    };

                    tx_receipts.push(receipt);
                    continue;
                }
            };
            // Commit changes after pre_dispatch_tx_hook
            batch_workspace = batch_workspace.checkpoint().to_revertable();

            let ctx = C::new(sender_address.clone());
            let tx_result = self.runtime.dispatch_call(msg, &mut batch_workspace, &ctx);

            let events = batch_workspace.take_events();
            let tx_effect = match tx_result {
                Ok(_) => TxEffect::Successful,
                Err(e) => {
                    debug!(
                        "Tx 0x{} was reverted error: {}",
                        hex::encode(raw_tx_hash),
                        e
                    );
                    // The transaction causing invalid state transition is reverted
                    // but we don't slash and we continue processing remaining transactions.
                    batch_workspace = batch_workspace.revert().to_revertable();
                    TxEffect::Reverted
                }
            };
            debug!("Tx {} effect: {:?}", hex::encode(raw_tx_hash), tx_effect);

            let receipt = TransactionReceipt {
                tx_hash: raw_tx_hash,
                body_to_save: None,
                events,
                receipt: tx_effect,
            };

            tx_receipts.push(receipt);
            // We commit after events have been extracted into receipt.
            batch_workspace = batch_workspace.checkpoint().to_revertable();

            // TODO: `panic` will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
            self.runtime
                .post_dispatch_tx_hook(&tx, &mut batch_workspace)
                .expect("Impossible happened: error in post_dispatch_tx_hook");
        }

        // TODO: calculate the amount based of gas and fees
        let sequencer_outcome = SequencerOutcome::Rewarded(0);

        if let Err(e) = self
            .runtime
            .end_blob_hook(sequencer_outcome.clone(), &mut batch_workspace)
        {
            // TODO: will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
            error!("Failed on `end_blob_hook`: {}", e);
        };

        self.checkpoint = Some(batch_workspace.checkpoint());
        BatchReceipt {
            batch_hash: blob.hash(),
            tx_receipts,
            inner: sequencer_outcome,
        }
    }

    fn end_slot(&mut self) -> (Self::StateRoot, Self::Witness) {
        let (cache_log, witness) = self.checkpoint.take().unwrap().freeze();
        let root_hash = self
            .current_storage
            .validate_and_commit(cache_log, &witness)
            .expect("jellyfish merkle tree update must succeed");
        (jmt::RootHash(root_hash), witness)
    }

    type SyncReceiptContents = SequencerOutcome;

    fn apply_sync_data_blob(
        &mut self,
        blob: &mut impl sov_rollup_interface::da::BlobTransactionTrait,
    ) -> sov_rollup_interface::stf::SyncReceipt<Self::SyncReceiptContents> {
        let mut batch_workspace = self
            .checkpoint
            .take()
            .expect("Working_set was initialized in begin_slot")
            .to_revertable();

        let address = match self.runtime.pre_blob_hook(blob, &mut batch_workspace) {
            Ok(address) => address,
            Err(e) => {
                info!("Sync pre-blob hook rejected: {:?}", e);
                return SyncReceipt {
                    blob_hash: blob.hash(),
                    inner: SequencerOutcome::Ignored,
                };
            }
        };

        let data = blob.data_mut();
        let mut contiguous_data = Vec::with_capacity(data.total_len());
        data.read_to_end(&mut contiguous_data)
            .expect("Reading from blob should succeed");

        let decoded = RT::decode_call(&contiguous_data);
        match decoded {
            Ok(call) => {
                // TODO: do something with this result
                let _ =
                    self.runtime
                        .dispatch_call(call, &mut batch_workspace, &Context::new(address));
            }
            Err(e) => {
                let sequencer_da_address: Vec<u8> = blob.sender().as_ref().to_vec();
                info!("Sync data blob decoding failed: {:?}", e);
                return SyncReceipt {
                    blob_hash: blob.hash(),
                    inner: SequencerOutcome::Slashed {
                        reason: SlashingReason::InvalidBatchEncoding,
                        sequencer_da_address,
                    },
                };
            }
        };
        // TODO: Make the reward sensible
        SyncReceipt {
            blob_hash: blob.hash(),
            inner: SequencerOutcome::Rewarded(0),
        }
    }
}
