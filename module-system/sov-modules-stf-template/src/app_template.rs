use std::marker::PhantomData;

use borsh::BorshDeserialize;
use sov_modules_api::{Context, DispatchCall};
use sov_rollup_interface::da::{BlobReaderTrait, CountedBufReader};
use sov_rollup_interface::stf::{BatchReceipt, TransactionReceipt};
use sov_rollup_interface::Buf;
use sov_state::StateCheckpoint;
use tracing::{debug, error};

use crate::tx_verifier::{verify_txs_stateless, TransactionAndRawHash};
use crate::{Batch, Runtime, SequencerOutcome, SlashingReason, TxEffect};

type ApplyBatchResult<T> = Result<T, ApplyBatchError>;

/// An implementation of the
/// [`StateTransitionFunction`](sov_rollup_interface::stf::StateTransitionFunction)
/// that is specifically designed to work with the module-system.
pub struct AppTemplate<C: Context, RT, Vm, B> {
    /// State storage used by the rollup.
    pub current_storage: C::Storage,
    /// The runtime includes all the modules that the rollup supports.
    pub runtime: RT,
    pub(crate) checkpoint: Option<StateCheckpoint<C::Storage>>,
    phantom_vm: PhantomData<Vm>,
    phantom_blob: PhantomData<B>,
}

pub(crate) enum ApplyBatchError {
    // Contains batch hash
    Ignored([u8; 32]),
    Slashed {
        // Contains batch hash
        hash: [u8; 32],
        reason: SlashingReason,
        sequencer_da_address: Vec<u8>,
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
            ApplyBatchError::Slashed {
                hash,
                reason,
                sequencer_da_address,
            } => BatchReceipt {
                batch_hash: hash,
                tx_receipts: Vec::new(),
                inner: SequencerOutcome::Slashed {
                    reason,
                    sequencer_da_address,
                },
            },
        }
    }
}

impl<C: Context, RT, Vm, B: BlobReaderTrait> AppTemplate<C, RT, Vm, B>
where
    RT: Runtime<C>,
{
    /// [`AppTemplate`] constructor.
    pub fn new(storage: C::Storage, runtime: RT) -> Self {
        Self {
            runtime,
            current_storage: storage,
            checkpoint: None,
            phantom_vm: PhantomData,
            phantom_blob: PhantomData,
        }
    }

    pub(crate) fn apply_blob(
        &mut self,
        blob: &mut B,
    ) -> ApplyBatchResult<BatchReceipt<SequencerOutcome, TxEffect>> {
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
            return Err(ApplyBatchError::Ignored(blob.hash()));
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

                return Err(ApplyBatchError::Slashed {
                    hash: blob.hash(),
                    reason,
                    sequencer_da_address,
                });
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
                    error!(
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
        Ok(BatchReceipt {
            batch_hash: blob.hash(),
            tx_receipts,
            inner: sequencer_outcome,
        })
    }

    // Do all stateless checks and data formatting, that can be results in sequencer slashing
    fn pre_process_batch(
        &self,
        blob_data: &mut CountedBufReader<impl Buf>,
    ) -> Result<
        (
            Vec<TransactionAndRawHash<C>>,
            Vec<<RT as DispatchCall>::Decodable>,
        ),
        SlashingReason,
    > {
        let batch = self.deserialize_batch(blob_data)?;
        debug!("Deserialized batch with {} txs", batch.txs.len());

        // Run the stateless verification, since it is stateless we don't commit.
        let txs = self.verify_txs_stateless(batch)?;

        let messages = self.decode_txs(&txs)?;

        Ok((txs, messages))
    }

    // Attempt to deserialize batch, error results in sequencer slashing.
    fn deserialize_batch(
        &self,
        blob_data: &mut CountedBufReader<impl Buf>,
    ) -> Result<Batch, SlashingReason> {
        match Batch::deserialize_reader(blob_data) {
            Ok(batch) => Ok(batch),
            Err(e) => {
                error!(
                    "Unable to deserialize batch provided by the sequencer {}",
                    e
                );
                Err(SlashingReason::InvalidBatchEncoding)
            }
        }
    }

    // Stateless verification of transaction, such as signature check
    // Single malformed transaction results in sequencer slashing.
    fn verify_txs_stateless(
        &self,
        batch: Batch,
    ) -> Result<Vec<TransactionAndRawHash<C>>, SlashingReason> {
        match verify_txs_stateless(batch.txs) {
            Ok(txs) => Ok(txs),
            Err(e) => {
                error!("Stateless verification error - the sequencer included a transaction which was known to be invalid. {}\n", e);
                Err(SlashingReason::StatelessVerificationFailed)
            }
        }
    }

    // Checks that runtime message can be decoded from transaction.
    // If a single message cannot be decoded, sequencer is slashed
    fn decode_txs(
        &self,
        txs: &[TransactionAndRawHash<C>],
    ) -> Result<Vec<<RT as DispatchCall>::Decodable>, SlashingReason> {
        let mut decoded_messages = Vec::with_capacity(txs.len());
        for TransactionAndRawHash { tx, raw_tx_hash } in txs {
            match RT::decode_call(tx.runtime_msg()) {
                Ok(msg) => decoded_messages.push(msg),
                Err(e) => {
                    error!("Tx 0x{} decoding error: {}", hex::encode(raw_tx_hash), e);
                    return Err(SlashingReason::InvalidTransactionEncoding);
                }
            }
        }
        Ok(decoded_messages)
    }
}
