use crate::{
    tx_verifier::{verify_txs_stateless, TransactionAndRawHash},
    Batch, SequencerOutcome, SlashingReason, TxEffect,
};
use borsh::BorshDeserialize;
use sov_modules_api::{
    hooks::{ApplyBlobHooks, TxHooks},
    Context, DispatchCall, Genesis,
};
use sov_rollup_interface::{
    da::{BlobTransactionTrait, BufWithCounter},
    stf::{BatchReceipt, TransactionReceipt},
    traits::BatchTrait,
    Buf,
};
use sov_state::{StateCheckpoint, WorkingSet};
use std::marker::PhantomData;
use tracing::{debug, error};

type Result<T> = std::result::Result<T, ApplyBatchError>;

pub struct AppTemplate<C: Context, RT, Vm> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    pub(crate) checkpoint: Option<StateCheckpoint<C::Storage>>,
    phantom_vm: PhantomData<Vm>,
}

pub(crate) enum ApplyBatchError {
    /// Contains batch hash
    Ignored([u8; 32]),
    Slashed {
        /// Contains batch hash
        hash: [u8; 32],
        reason: SlashingReason,
    },
}

impl From<ApplyBatchError> for BatchReceipt<SequencerOutcome, TxEffect> {
    fn from(value: ApplyBatchError) -> Self {
        match value {
            ApplyBatchError::Ignored(hash) => BatchReceipt {
                batch_hash: hash,
                tx_receipts: Vec::new(),
                inner: SequencerOutcome::Ignored,
            },
            ApplyBatchError::Slashed { hash, reason } => BatchReceipt {
                batch_hash: hash,
                tx_receipts: Vec::new(),
                inner: SequencerOutcome::Slashed(reason),
            },
        }
    }
}

impl<C: Context, RT, Vm> AppTemplate<C, RT, Vm>
where
    RT: DispatchCall<Context = C>
        + Genesis<Context = C>
        + TxHooks<Context = C>
        + ApplyBlobHooks<Context = C, BlobResult = SequencerOutcome>,
{
    fn init_sequencer_and_get_working_set(
        &mut self,
        blob: &mut impl BlobTransactionTrait,
    ) -> Result<WorkingSet<C::Storage>> {
        debug!(
            "Applying batch from sequencer: 0x{}",
            hex::encode(blob.sender())
        );
        let mut batch_workspace = self
            .checkpoint
            .take()
            .expect("Working_set was initialized in begin_slot")
            .to_revertable();

        if let Err(e) = self.runtime.begin_blob_hook(
            blob.sender().as_ref(),
            &blob.data_mut().reader(),
            &mut batch_workspace,
        ) {
            error!(
                "Error: The transaction was rejected by the 'enter_apply_blob' hook. Skipping batch without slashing the sequencer: {}",
                e
            );
            self.checkpoint = Some(batch_workspace.revert());
            return Err(ApplyBatchError::Ignored(blob.hash()));
        }

        Ok(batch_workspace)
    }

    fn deserialize_batch(
        &mut self,
        batch_workspace: WorkingSet<C::Storage>,
        blob_data: &mut BufWithCounter<impl Buf>,
        blob_hash: [u8; 32],
    ) -> Result<(WorkingSet<C::Storage>, Batch)> {
        match Batch::deserialize_reader(&mut blob_data.reader()) {
            Ok(batch) => Ok((batch_workspace, batch)),
            Err(e) => {
                error!(
                    "Unable to deserialize batch provided by the sequencer {}",
                    e
                );
                self.checkpoint = Some(batch_workspace.revert());
                Err(ApplyBatchError::Slashed {
                    hash: blob_hash,
                    reason: SlashingReason::InvalidBatchEncoding,
                })
            }
        }
    }

    fn verify_txs_stateless(
        &mut self,
        batch_workspace: WorkingSet<C::Storage>,
        batch: Batch,
        blob_hash: [u8; 32],
    ) -> Result<(WorkingSet<C::Storage>, Vec<TransactionAndRawHash<C>>)> {
        // Run the stateless verification, since it is stateless we don't commit.
        match verify_txs_stateless(batch.take_transactions()) {
            Ok(txs) => Ok((batch_workspace, txs)),
            Err(e) => {
                // Revert on error
                self.checkpoint = Some(batch_workspace.revert());
                error!("Stateless verification error - the sequencer included a transaction which was known to be invalid. {}\n", e);
                Err(ApplyBatchError::Slashed {
                    hash: blob_hash,
                    reason: SlashingReason::StatelessVerificationFailed,
                })
            }
        }
    }

    fn execute_txs(
        &mut self,
        mut batch_workspace: WorkingSet<C::Storage>,
        txs: Vec<TransactionAndRawHash<C>>,
        blob_hash: [u8; 32],
    ) -> Result<(WorkingSet<C::Storage>, Vec<TransactionReceipt<TxEffect>>)> {
        let mut tx_receipts = Vec::with_capacity(txs.len());

        // Process transactions in a loop, commit changes after every step of the loop.
        for TransactionAndRawHash { tx, raw_tx_hash } in txs {
            // Run the stateful verification, possibly modifies the state.
            let sender_address = match self
                .runtime
                .pre_dispatch_tx_hook(tx.clone(), &mut batch_workspace)
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

            match RT::decode_call(tx.runtime_msg()) {
                Ok(msg) => {
                    let ctx = C::new(sender_address.clone());
                    let tx_result = self.runtime.dispatch_call(msg, &mut batch_workspace, &ctx);

                    self.runtime
                        .post_dispatch_tx_hook(&tx, &mut batch_workspace)
                        .expect("Impossible happened: error in post_dispatch_tx_hook");

                    let tx_effect = match tx_result {
                        Ok(_) => TxEffect::Successful,
                        Err(e) => {
                            debug!(
                                "Tx 0x{} was reverted error: {}",
                                hex::encode(raw_tx_hash),
                                e
                            );

                            // The transaction causing invalid state transition is reverted but we don't slash and we continue
                            // processing remaining transactions.
                            batch_workspace = batch_workspace.revert().to_revertable();
                            TxEffect::Reverted
                        }
                    };

                    let receipt = TransactionReceipt {
                        tx_hash: raw_tx_hash,
                        body_to_save: None,
                        events: batch_workspace.take_events(),
                        receipt: tx_effect,
                    };

                    tx_receipts.push(receipt);
                }
                Err(e) => {
                    // If the serialization is invalid, the sequencer is malicious. Slash them (we don't run exit_apply_batch here)
                    let batch_workspace = batch_workspace.revert();
                    self.checkpoint = Some(batch_workspace);
                    error!("Tx 0x{} decoding error: {}", hex::encode(raw_tx_hash), e);

                    return Err(ApplyBatchError::Slashed {
                        hash: blob_hash,
                        reason: SlashingReason::InvalidTransactionEncoding,
                    });
                }
            };
            // commit each step of the loop
            batch_workspace = batch_workspace.checkpoint().to_revertable();
        }
        Ok((batch_workspace, tx_receipts))
    }

    pub fn new(storage: C::Storage, runtime: RT) -> Self {
        Self {
            runtime,
            current_storage: storage,
            checkpoint: None,
            phantom_vm: PhantomData,
        }
    }

    pub(crate) fn apply_blob(
        &mut self,
        blob: &mut impl BlobTransactionTrait,
    ) -> Result<BatchReceipt<SequencerOutcome, TxEffect>> {
        let mut batch_workspace = self.init_sequencer_and_get_working_set(blob)?;

        // TODO: don't ignore these events.
        // https://github.com/Sovereign-Labs/sovereign/issues/350
        let _ = batch_workspace.take_events();

        let blob_hash = blob.hash();

        // Commit changes.
        batch_workspace = batch_workspace.checkpoint().to_revertable();

        // This function consumes the blob data
        let (batch_workspace, batch) =
            self.deserialize_batch(batch_workspace, blob.data_mut(), blob_hash)?;

        debug!("Deserialized batch with {} txs", batch.txs.len());

        // Run the stateless verification, since it is stateless we don't commit.
        let (batch_workspace, txs) =
            self.verify_txs_stateless(batch_workspace, batch, blob_hash)?;

        let (mut batch_workspace, tx_receipts) =
            self.execute_txs(batch_workspace, txs, blob_hash)?;

        // TODO: calculate the amount based of gas and fees
        let batch_receipt_contents = SequencerOutcome::Rewarded(0);
        self.runtime
            .end_blob_hook(batch_receipt_contents, &mut batch_workspace)
            .expect("Impossible happened: error in exit_apply_batch");

        self.checkpoint = Some(batch_workspace.checkpoint());
        Ok(BatchReceipt {
            batch_hash: blob_hash,
            tx_receipts,
            inner: batch_receipt_contents,
        })
    }
}
