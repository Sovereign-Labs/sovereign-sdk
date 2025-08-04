use sov_modules_api::capabilities::{AuthenticationError, AuthenticationOutput, FatalError};
use sov_modules_api::transaction::AuthenticatedTransactionData;
use sov_modules_api::{
    BatchSequencerReceipt, Context, DispatchCall, Error, IgnoredTransactionReceipt, Spec,
    StateProvider, TransactionReceipt, TxScratchpad, WorkingSet, *,
};
use sov_rollup_interface::TxHash;
use tracing::{debug, info};

use super::registered::IncrementalBatchReceipt;
use crate::stf_blueprint::convert_to_runtime_events;
use crate::{ApplyTxResult, Runtime, TxReceiptContents};

/// Applies a single transaction to the current state. In normal execution, we commit twice times execution:
/// 1. After the pre-dispatch hook. This ensures that the gas charges are paid even if the transaction fails later during execution
/// 2. After the post-dispatch hook. This ensures that the transaction can be reverted by the post-dispatch hook if desired.
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker(raw_tx_hash))]
#[allow(clippy::too_many_arguments)]
pub fn apply_tx<S, RT, I>(
    runtime: &mut RT,
    ctx: &Context<S>,
    tx: &AuthenticatedTransactionData<S>,
    raw_tx_hash: TxHash,
    raw_tx: FullyBakedTx,
    message: <RT as DispatchCall>::Decodable,
    mut working_set: WorkingSet<S, I>,
) -> (ApplyTxResult<S>, TxScratchpad<S, I>)
where
    S: Spec,
    RT: Runtime<S>,
    I: StateProvider<S>,
{
    let tx_result = attempt_tx(tx, message, ctx, runtime, &mut working_set);
    let (tx_scratchpad, receipt, transaction_consumption) = match tx_result {
        Ok(_) => {
            let (tx_scratchpad, transaction_consumption, events) = working_set.finalize();
            let gas_used = transaction_consumption.base_fee();

            (
                tx_scratchpad,
                TransactionReceipt {
                    tx_hash: raw_tx_hash,
                    body_to_save: Some(raw_tx.data),
                    events: convert_to_runtime_events::<S, RT>(events, raw_tx_hash.into()),
                    receipt: TxEffect::Successful(SuccessfulTxContents {
                        gas_used: gas_used.clone(),
                        // TODO: Add additional fields here for...
                        // - sender
                        // - authorizer_id (i.e. Ethereum or Solana)
                        // - credential
                        // - nonce
                        // - etc.
                    }),
                },
                transaction_consumption,
            )
        }
        Err(error) => {
            // It's expected that transactions will revert, so we log them at the info level.
            info!(
                %error,
                %raw_tx_hash,
                "Tx was reverted",
            );
            // the transaction causing invalid state transition is reverted,
            // but we don't slash and continue processing remaining transactions.
            // working_set.revert_in_place();
            let (tx_scratchpad, transaction_consumption) = working_set.revert();

            let receipt = TransactionReceipt {
                tx_hash: raw_tx_hash,
                body_to_save: Some(raw_tx.data),
                events: vec![], // As in Ethereum, reverted transactions don't emit events
                receipt: TxEffect::Reverted(RevertedTxContents {
                    gas_used: transaction_consumption.base_fee().clone(),
                    reason: error,
                }),
            };

            (tx_scratchpad, receipt, transaction_consumption)
        }
    };

    (
        ApplyTxResult::<S> {
            transaction_consumption,
            receipt,
        },
        tx_scratchpad,
    )
}

fn attempt_tx<S: Spec, RT: Runtime<S>, I: StateProvider<S>>(
    tx: &AuthenticatedTransactionData<S>,
    message: <RT as DispatchCall>::Decodable,
    ctx: &Context<S>,
    runtime: &mut RT,
    state: &mut WorkingSet<S, I>,
) -> Result<(), Error> {
    runtime.pre_dispatch_tx_hook(tx, state)?;

    runtime.dispatch_call(message, state, ctx)?;

    runtime.post_dispatch_tx_hook(tx, ctx, state)?;

    Ok(())
}

/// Error during the pre-flight checks before the transaction is executed.
#[derive(Debug, thiserror::Error)]
pub enum PreExecError {
    /// Sequencer error.
    #[error("Sequencer error")]
    SequencerError(#[source] anyhow::Error),
    /// Invalid transaction from registered sequencer.
    #[error("Invalid transaction from registered sequencer")]
    AuthError(#[source] AuthenticationError),
}

/// Alias for `AuthenticationOutput`.
pub type AuthTxOutput<S, R> = AuthenticationOutput<S, <R as DispatchCall>::Decodable>;

/// The receipt for a batch using the STF blueprint.
pub type BatchReceipt<S> =
    sov_rollup_interface::stf::BatchReceipt<BatchSequencerReceipt<S>, TxReceiptContents<S>>;

/// Returns the gas used by a transaction from its receipt.
pub fn get_gas_used<S: Spec>(receipt: &TransactionReceipt<S>) -> S::Gas {
    match &receipt.receipt {
        TxEffect::Successful(ref successful) => successful.gas_used.clone(),
        TxEffect::Reverted(ref reverted) => reverted.gas_used.clone(),
        TxEffect::Skipped(skipped) => skipped.gas_used.clone(),
    }
}

pub(crate) fn create_tx_receipt<S: Spec>(
    skipped: SkippedTxContents<S>,
    raw_tx_hash: TxHash,
    raw_tx_body: Vec<u8>,
) -> TransactionReceipt<S> {
    info!(
        error = %skipped.error,
        raw_tx_hash = %raw_tx_hash,
        "An error occurred while processing a transaction. The transaction was not executed and skipped",
    );

    TransactionReceipt {
        tx_hash: raw_tx_hash,
        body_to_save: Some(raw_tx_body),
        events: Vec::new(),
        receipt: TxEffect::Skipped(skipped),
    }
}

pub(crate) struct BatchReceiptContents<'a, S: Spec> {
    pub tx_receipts: &'a Vec<TransactionReceipt<S>>,
    pub ignored_tx_receipts: &'a Vec<IgnoredTransactionReceipt<TxReceiptContents<S>>>,
    pub inner: &'a BatchSequencerReceipt<S>,
}

impl<'a, S: Spec> From<&'a IncrementalBatchReceipt<S>> for BatchReceiptContents<'a, S> {
    fn from(value: &'a IncrementalBatchReceipt<S>) -> Self {
        Self {
            tx_receipts: &value.tx_receipts,
            ignored_tx_receipts: &value.ignored_tx_receipts,
            inner: &value.inner,
        }
    }
}

impl<'a, S: Spec> From<&'a BatchReceipt<S>> for BatchReceiptContents<'a, S> {
    fn from(value: &'a BatchReceipt<S>) -> Self {
        Self {
            tx_receipts: &value.tx_receipts,
            ignored_tx_receipts: &value.ignored_tx_receipts,
            inner: &value.inner,
        }
    }
}

pub(crate) fn apply_batch_logs<'a, S: Spec>(
    batch_receipt: impl Into<BatchReceiptContents<'a, S>>,
    blob_idx: usize,
) {
    let batch_receipt = batch_receipt.into();

    debug!(
        blob_idx,
        num_txs = batch_receipt.tx_receipts.len(),
        num_ignored_txs = batch_receipt.ignored_tx_receipts.len(),
        sequencer_outcome = %batch_receipt.inner,
        "Applied blob and got the sequencer outcome"
    );

    for (i, tx_receipt) in batch_receipt.tx_receipts.iter().enumerate() {
        debug!(
            tx_idx = i,
            tx_hash = %tx_receipt.tx_hash,
            receipt = ?tx_receipt.receipt,
            gas_used = %get_gas_used(tx_receipt),
            "Tx receipt"
        );
    }

    for tx_receipt in batch_receipt.ignored_tx_receipts.iter() {
        debug!(
            receipt = ?tx_receipt,
            gas_used = %tx_receipt.ignored.gas_used,
            "Ignored Tx receipt"
        );
    }
}

/// The output of the authentication phase.
pub enum ValidatedAuthOutput<S: Spec, R: Runtime<S>> {
    /// Transaction data after the authentication phase.
    Valid(AuthTxOutput<S, R>),
    /// Transaction failed authentication.
    Invalid {
        /// Transaction hash.
        tx_hash: TxHash,
        /// Authentication error.
        error: FatalError,
    },
}

impl<S: Spec, R: Runtime<S>> ValidatedAuthOutput<S, R> {
    /// Get hash of the Validated Auth Output.
    pub fn hash(&self) -> TxHash {
        match self {
            ValidatedAuthOutput::Valid(valid) => valid.0.raw_tx_hash,
            ValidatedAuthOutput::Invalid { tx_hash, error: _ } => *tx_hash,
        }
    }
}
