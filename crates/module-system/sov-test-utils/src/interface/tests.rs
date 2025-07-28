use borsh::BorshDeserialize;
use sov_modules_api::{
    Amount, ApiStateAccessor, BatchSequencerReceipt, DaSpec, ProofReceipt, Runtime,
    RuntimeEventProcessor, Spec, TransactionReceipt, TxEffect, *,
};
use sov_state::{Storage, StorageProof};

use super::{BatchType, ProofInput, TransactionType};
use crate::runtime::BlobInfo;

type TestAssertion<Context, S> = Box<dyn FnOnce(Context, &mut ApiStateAccessor<S>)>;
type BatchReceipt<S> =
    sov_modules_api::BatchReceipt<BatchSequencerReceipt<S>, TxReceiptContents<S>>;

/// Context that is passed to [`TransactionTestCase::assert`] to check the outcome of a test.
pub struct TransactionAssertContext<S: Spec, RT: RuntimeEventProcessor> {
    /// The gas used to execute the transaction, expressed in gas tokens.
    pub gas_value_used: Amount,
    /// The events raised by the transaction.
    ///
    /// The RuntimeEvent can be checked for specific module events, using the `sov_bank` module
    /// as an example below.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let context = TransactionAssertContext { .. };
    /// let runtime_event = context.events[0];
    /// matches!(
    ///     &runtime_event,
    ///     GeneratedRuntimeEvent::Bank(
    ///         sov_bank::event::Event::TokenCreated { .. }
    /// ));
    /// ```
    ///
    pub events: Vec<RT::RuntimeEvent>,
    /// The metadata about the blob that contained the transaction
    pub blob_info: BlobInfo,
    /// The outcome of the transaction.
    pub tx_receipt: TxEffect<S>,
}

impl<S: Spec, RT: RuntimeEventProcessor> TransactionAssertContext<S, RT> {
    /// Creates a [`TransactionAssertContext`] from the given [`TransactionReceipt`].
    pub fn from_receipt<Da: DaSpec>(
        receipt: TransactionReceipt<S>,
        blob_info: BlobInfo,
        gas_value_used: Amount,
    ) -> Self {
        let events = receipt
            .events
            .into_iter()
            .map(|stored_event| {
                <RT as RuntimeEventProcessor>::RuntimeEvent::deserialize(
                    &mut stored_event.value().inner().as_slice(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        TransactionAssertContext {
            tx_receipt: receipt.receipt,
            events,
            blob_info,
            gas_value_used,
        }
    }
}

/// A closure used to assert the outcome of a [`TransactionTestCase`].
pub type TransactionTestAssert<RT, S> = TestAssertion<TransactionAssertContext<S, RT>, S>;

/// A test case that applies the provided input and asserts the result.
pub struct TransactionTestCase<RT: Runtime<S>, S: Spec> {
    /// Input transaction to execute.
    pub input: TransactionType<RT, S>,
    /// Closure used to assert the outcome of the input application
    /// to the rollup state.
    pub assert: TransactionTestAssert<RT, S>,
}

/// Context that is passed to [`BatchTestCase::assert`] to check the outcome of a test.
pub struct BatchAssertContext<S: Spec> {
    /// The DA address of the sender of the batch.
    pub sender_da_address: <S::Da as DaSpec>::Address,
    /// The outcome of the batch submission
    ///
    /// This can be [`None`] if the batch was dropped before it was executed,
    /// this can happen if the sender was not a registered sequencer.
    pub batch_receipt: Option<BatchReceipt<S>>,
}

/// A closure used to assert the outcome of a [`BatchTestCase`].
pub type BatchTestAssert<S> = TestAssertion<BatchAssertContext<S>, S>;

/// A test case that applies the provided batch input and asserts the result.
pub struct BatchTestCase<RT: Runtime<S>, S: Spec> {
    /// Input to execute as part of the batch.
    pub input: BatchType<RT, S>,
    /// Closure used to assert the outcome of applying the batch to the rollup.
    pub assert: BatchTestAssert<S>,
}

/// Context that is passed to [`ProofTestCase::assert`] to check the outcome of a test.
pub struct ProofAssertContext<S: Spec> {
    /// The outcome of the proof submission.
    ///
    /// This can be [`None`] if the proof was dropped before it was executed,
    /// this can happen if the proof was malformed by the prover. Generally this should always be
    /// present.
    #[allow(clippy::type_complexity)]
    pub proof_receipt: Option<
        ProofReceipt<
            <S as Spec>::Address,
            S::Da,
            <<S as Spec>::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
    >,

    /// The gas used to verify the proof.
    pub gas_value_used: Amount,
}

/// A closure used to assert the outcome of a [`ProofTestCase`].
pub type ProofTestAssert<S> = TestAssertion<ProofAssertContext<S>, S>;

/// A test case that applies the provided proof input and asserts the result.
pub struct ProofTestCase<S: Spec> {
    /// Input for the test case.
    pub input: ProofInput,
    /// Assertion for the test case.
    pub assert: ProofTestAssert<S>,
}
