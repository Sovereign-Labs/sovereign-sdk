//! Web3 transaction building utilities for Sovereign SDK.
//!
//! This crate provides a high-level interface for building and signing transactions
//! in the Sovereign SDK ecosystem. It includes a builder pattern for constructing
//! transactions with various parameters like fees, gas limits, and uniqueness data.

use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::{CallMessage, CryptoSpec, RuntimeDiscriminant, UnmanagedRuntimeCall};

pub use sov_modules_api::capabilities::UniquenessData;
pub use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
pub use sov_modules_api::{Amount, Spec};

/// Errors that can occur when building transactions.
#[derive(thiserror::Error, Debug)]
pub enum TransactionBuilderError {
    /// The provided private key is invalid or malformed.
    #[error("The private key was invalid")]
    PrivateKeyInvalid,
}

/// Trait for providing chain-specific hash values.
///
/// This trait must be implemented by types that need to provide
/// a unique 32-byte hash identifying a specific blockchain.
pub trait ChainHash {
    /// Returns the 32-byte hash that uniquely identifies the chain.
    fn chain_hash() -> [u8; 32];
}

/// Default maximum priority fee in basis points (0).
pub const DEFAULT_MAX_PRIORITY_FEE_BIPS: PriorityFeeBips = PriorityFeeBips::ZERO;

/// Default maximum fee amount (100,000,000 units).
pub const DEFAULT_MAX_FEE: Amount = Amount(100000000);

/// Generates default uniqueness data based on current timestamp.
///
/// Creates a `UniquenessData::Generation` variant using the current
/// Unix timestamp in seconds.
///
/// # Panics
///
/// Panics if the system clock is set to a time before the Unix epoch.
pub fn default_uniqueness() -> UniquenessData {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    UniquenessData::Generation(now)
}

/// A builder for constructing transactions with customizable parameters.
///
/// The `TransactionBuilder` provides a fluent interface for building transactions
/// with various optional parameters like fees, gas limits, and uniqueness data.
/// Once configured, it can produce either unsigned transactions or fully signed transactions.
///
/// # Type Parameters
///
/// * `S` - The specification type that defines the blockchain's configuration
/// * `C` - A type implementing `ChainHash` to provide the chain identifier
/// * `M` - The call message type that implements `CallMessage` and `RuntimeDiscriminant`
///
/// # Examples
///
/// ```ignore
/// let builder = TransactionBuilder::new(my_call)
///     .max_fee(1000u64)
///     .priority_fee_bips(100u16)
///     .uniqueness(my_uniqueness_data);
///
/// let unsigned_tx = builder.build()?;
/// ```
pub struct TransactionBuilder<S: Spec, C: ChainHash, M: CallMessage + RuntimeDiscriminant> {
    call: M,
    uniqueness: Option<UniquenessData>,
    priority_fee_bips: Option<PriorityFeeBips>,
    max_fee: Option<Amount>,
    gas_limit: Option<Option<S::Gas>>,
    _phantom: std::marker::PhantomData<C>,
}

impl<S: Spec, C: ChainHash, M: CallMessage + RuntimeDiscriminant> TransactionBuilder<S, C, M> {
    /// Creates a new transaction builder with the specified call message.
    ///
    /// All optional parameters are initially unset and will use default values
    /// when the transaction is built.
    ///
    /// # Arguments
    ///
    /// * `call` - The call message to include in the transaction
    pub fn new(call: M) -> Self {
        Self {
            call,
            uniqueness: None,
            priority_fee_bips: None,
            max_fee: None,
            gas_limit: None,
            _phantom: Default::default(),
        }
    }

    /// Sets the priority fee in basis points.
    ///
    /// If not set, defaults to [`DEFAULT_MAX_PRIORITY_FEE_BIPS`].
    ///
    /// # Arguments
    ///
    /// * `fee` - The priority fee amount in basis points
    pub fn priority_fee_bips<T: Into<PriorityFeeBips>>(mut self, fee: T) -> Self {
        self.priority_fee_bips = Some(fee.into());
        self
    }

    /// Sets the maximum fee willing to be paid for the transaction.
    ///
    /// This represents the total maximum amount that can be charged for executing
    /// the transaction. If not set, defaults to [`DEFAULT_MAX_FEE`].
    ///
    /// # Arguments
    ///
    /// * `max_fee` - The maximum fee amount
    pub fn max_fee<T: Into<Amount>>(mut self, max_fee: T) -> Self {
        self.max_fee = Some(max_fee.into());
        self
    }

    /// Sets the uniqueness data for the transaction.
    ///
    /// Uniqueness data helps prevent transaction replay attacks by ensuring
    /// each transaction is unique. If not set, defaults to timestamp-based
    /// uniqueness via [`default_uniqueness`].
    ///
    /// # Arguments
    ///
    /// * `uniqueness` - The uniqueness data to use
    pub fn uniqueness(mut self, uniqueness: UniquenessData) -> Self {
        self.uniqueness = Some(uniqueness);
        self
    }

    /// Sets the gas limit for the transaction.
    ///
    /// The gas limit defines the maximum amount of computational work
    /// the transaction is allowed to perform. Pass `None` for unlimited gas,
    /// or `Some(limit)` to set a specific limit.
    ///
    /// # Arguments
    ///
    /// * `gas_limit` - The gas limit (None for unlimited, Some(limit) for a specific limit)
    pub fn gas_limit(mut self, gas_limit: Option<S::Gas>) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Builds an unsigned transaction with the configured parameters.
    ///
    /// Uses default values for any parameters that were not explicitly set:
    /// - Priority fee: [`DEFAULT_MAX_PRIORITY_FEE_BIPS`]
    /// - Max fee: [`DEFAULT_MAX_FEE`]
    /// - Gas limit: `None` (unlimited)
    /// - Uniqueness: Generated via [`default_uniqueness`]
    ///
    /// # Returns
    ///
    /// Returns an `UnsignedTransaction` that can be signed later, or an error
    /// if the transaction could not be constructed.
    pub fn build(
        self,
    ) -> Result<UnsignedTransaction<UnmanagedRuntimeCall<M>, S>, TransactionBuilderError> {
        let priority_fee = self
            .priority_fee_bips
            .unwrap_or(DEFAULT_MAX_PRIORITY_FEE_BIPS);
        let max_fee = self.max_fee.unwrap_or(DEFAULT_MAX_FEE);
        let gas_limit = self.gas_limit.unwrap_or(None);
        let uniqueness = self.uniqueness.unwrap_or_else(default_uniqueness);

        Ok(UnsignedTransaction::new(
            self.call,
            config_chain_id(),
            priority_fee,
            max_fee,
            uniqueness,
            gas_limit,
        ))
    }

    /// Builds and signs a transaction in one step.
    ///
    /// This is a convenience method that builds the transaction with the configured
    /// parameters and immediately signs it with the provided private key.
    ///
    /// # Arguments
    ///
    /// * `private_key` - The private key bytes used to sign the transaction
    ///
    /// # Returns
    ///
    /// Returns a fully signed `Transaction` ready for submission, or an error
    /// if the transaction could not be built or the private key is invalid.
    ///
    /// # Errors
    ///
    /// * [`TransactionBuilderError::PrivateKeyInvalid`] - If the private key bytes
    ///   cannot be converted to the expected private key type
    pub fn build_and_sign(
        self,
        private_key: &[u8],
    ) -> Result<Transaction<UnmanagedRuntimeCall<M>, S>, TransactionBuilderError> {
        let typed_private_key =
            <S::CryptoSpec as CryptoSpec>::PrivateKey::try_from(private_key.to_vec())
                .map_err(|_| TransactionBuilderError::PrivateKeyInvalid)?;
        Ok(self.build()?.sign(&typed_private_key, &C::chain_hash()))
    }
}
