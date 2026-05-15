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

/// Structured error returned when a transaction's uniqueness check fails.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CheckUniquenessError {
    /// The generation number is older than the sequencer's acceptance window.
    #[error("bad generation: latest known generation is {latest_generation}, provided {provided_generation} is too old")]
    BadGeneration {
        /// The latest generation the sequencer has seen for this credential.
        latest_generation: u64,
        /// The generation provided in the transaction.
        provided_generation: u64,
    },
    /// The transaction hash was already seen at this generation.
    #[error("duplicate transaction at generation {generation}")]
    DuplicateGeneration {
        /// The generation at which the duplicate was detected.
        generation: u64,
    },
    /// Too many transactions at the current generation; the credential must increment it.
    #[error("too many transactions at generation {current_generation}: increment generation to {next_valid_generation}")]
    GenerationCapacityExceeded {
        /// The generation that is full.
        current_generation: u64,
        /// The minimum generation value that will be accepted next.
        next_valid_generation: u64,
    },
    /// The nonce was not the expected next value.
    #[error("bad nonce: expected {expected_nonce}, provided {provided_nonce}")]
    BadNonce {
        /// The nonce the sequencer expected.
        expected_nonce: u64,
        /// The nonce provided in the transaction.
        provided_nonce: u64,
    },
    /// The nonce was below the minimum accepted value (warm-up / non-consecutive mode).
    #[error("nonce too low: minimum {minimum_nonce}, provided {provided_nonce}")]
    NonceTooLow {
        /// The minimum nonce the sequencer will accept.
        minimum_nonce: u64,
        /// The nonce provided in the transaction.
        provided_nonce: u64,
    },
    /// An unexpected internal error occurred during the uniqueness check.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for CheckUniquenessError {
    fn from(e: anyhow::Error) -> Self {
        CheckUniquenessError::Internal(e.to_string())
    }
}

/// The transaction processing error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum TxProcessingError {
    /// Transaction authentication failed.
    #[error(" Transaction authentication failed {0}.")]
    AuthenticationFailed(String),
    /// The uniqueness check failed.
    #[error("The uniqueness check failed. Reason: {0}.")]
    CheckUniquenessFailed(CheckUniquenessError),
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
