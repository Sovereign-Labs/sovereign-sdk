pub use crate::common::ModuleError as Error;
use crate::Spec;

/// The receipt type for a transaction using the STF blueprint.
pub type TransactionReceipt<S> =
    sov_rollup_interface::stf::TransactionReceipt<TxReceiptContents<S>>;

/// The effect of a batch using the STF blueprint.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct TxReceiptContents<S>(std::marker::PhantomData<S>);

/// The effect of a transaction using the STF blueprint.
pub type TxEffect<S> = sov_rollup_interface::stf::TxEffect<TxReceiptContents<S>>;

impl<S: Spec> sov_rollup_interface::stf::TxReceiptContents for TxReceiptContents<S> {
    type Skipped = SkippedTxContents<S>;
    type Reverted = RevertedTxContents<S>;
    type Successful = SuccessfulTxContents<S>;
    type Ignored = IgnoredTxContents<S>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, thiserror::Error)]
/// The contents of the receipt for a reverted transaction
pub struct RevertedTxContents<S: Spec> {
    /// The gas consumed by the transaction
    pub gas_used: S::Gas,
    /// The reason the tx reverted.
    pub reason: Error,
}

impl<S: Spec> PartialEq for RevertedTxContents<S> {
    fn eq(&self, other: &Self) -> bool {
        self.gas_used == other.gas_used && self.reason == other.reason
    }
}
impl<S: Spec> Eq for RevertedTxContents<S> {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, thiserror::Error)]
/// The contents of the receipt for a successful transaction
pub struct SuccessfulTxContents<S: Spec> {
    /// The gas consumed by the transaction
    pub gas_used: S::Gas,
}

impl<S: Spec> PartialEq for SuccessfulTxContents<S> {
    fn eq(&self, other: &Self) -> bool {
        self.gas_used == other.gas_used
    }
}
impl<S: Spec> Eq for SuccessfulTxContents<S> {}

/// Ignored transactions consume gas but do not otherwise impact the state of the rollup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, thiserror::Error, Eq, PartialEq)]
pub struct IgnoredTxContents<S: Spec> {
    /// The gas consumed by the transaction
    pub gas_used: S::Gas,
    /// Index in the batch.
    pub index: usize,
}

/// The contents of the receipt for a skipped transaction
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct SkippedTxContents<S: Spec> {
    /// The gas consumed by the transaction.
    pub gas_used: S::Gas,
    /// Reason why the transaction was skipped.
    pub error: TxProcessingError,
}

impl<S: Spec> PartialEq for SkippedTxContents<S> {
    fn eq(&self, other: &Self) -> bool {
        self.gas_used == other.gas_used && self.error == other.error
    }
}
impl<S: Spec> Eq for SkippedTxContents<S> {}

/// The transaction processing error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum TxProcessingError {
    /// Transaction authentication failed.
    #[error(" Transaction authentication failed {0}.")]
    AuthenticationFailed(String),
    /// The uniqueness check failed.
    #[error("The uniqueness check failed. Reason: {0}.")]
    CheckUniquenessFailed(String),
    /// Impossible to reserve gas for the transaction to be executed.
    #[error("Impossible to reserve gas for the transaction to be executed, reason: {0}.")]
    CannotReserveGas(String),
    /// Impossible to resolve the context of the transaction.
    #[error("Impossible to resolve the context of the transaction, reason: {0}.")]
    CannotResolveContext(String),
    /// Rejected by a pre-flight check.
    #[error("The transaction was rejected by a pre-flight check.")]
    RejectedByPreFlight,
    /// Failed to mark transaction.
    #[error("Failed to mark transaction, reason: {0}.")]
    MarkTxAttemptedFailed(String),
    /// The transaction ran out of gas
    #[error("The transaction ran out of gas, reason: {0}.")]
    OutOfGas(String),
}
