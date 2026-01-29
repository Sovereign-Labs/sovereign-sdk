//! Module system runtime types and traits
pub mod capabilities;

#[cfg(feature = "native")]
use std::io;

use borsh::{BorshDeserialize, BorshSerialize};
use capabilities::{HasCapabilities, HasKernel, TransactionAuthenticator};
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use sov_rollup_interface::stf::GenesisParams;
#[cfg(feature = "native")]
use sov_state::pinned_cache::PinnedCache;
#[cfg(feature = "native")]
use sov_state::User;

#[cfg(feature = "native")]
use crate::hooks::FinalizeHook;
use crate::hooks::{BlockHooks, TxHooks};
use crate::transaction::TransactionCallable;
use crate::Context;
use crate::{DispatchCall, Genesis, RuntimeEventProcessor, Spec};
#[cfg(feature = "native")]
use crate::{FullyBakedTx, StateReader};

/// Flag indicating what mode the rollup is operating in.
#[derive(
    BorshDeserialize, BorshSerialize, Serialize, Deserialize, Debug, PartialEq, Eq, Copy, Clone,
)]
#[serde(rename_all = "snake_case")]
pub enum OperatingMode {
    /// The rollup is currently executing in optimistic mode.
    Optimistic,
    /// The rollup is currently executing in zk mode.
    Zk,
    /// The rollup is currently executing in operator mode.
    Operator,
}

/// This trait defines an interface to pass runtime configuration values for modules.
/// This is configuration that is ran off-chain, for example metric gathering configuration.
/// This is distinct from genesis configuration, which is passed to the runtime at genesis time
/// and stored on-chain.
///
/// This configuration and function does not need to be deterministic, as it is not executed
/// on-chain.
pub trait ModuleExecutionConfig {
    /// Input type for configuration.
    /// This could be a config struct that holds sub configuration for each runtime module
    /// that requires such configuration.
    type Input: Clone + Send + Sync;

    /// Execute configuration for modules.
    /// This function is called once at runtime startup and will typically pass specific module
    /// configuration values to each module that requires it.
    fn configure(_input: &Self::Input) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// Allow assigning `()` as the ModuleExecutionConfig when no configuration is needed.
// Acts as a no-op implementation.
impl ModuleExecutionConfig for () {
    type Input = ();

    fn configure(_input: &Self::Input) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

#[cfg(feature = "native")]
/// This trait has to be implemented by a runtime in order to be used in `StfBlueprint`.
///
/// The `TxHooks` implementation sets up a transaction context based on the height at which it is
/// to be executed.
pub trait Runtime<S: Spec>:
    DispatchCall<Spec = S>
    + HasCapabilities<S>
    + HasKernel<S>
    + Genesis<Spec = S, Config = Self::GenesisConfig>
    + TxHooks<Spec = S>
    + BlockHooks<Spec = S>
    + FinalizeHook<Spec = S>
    + Default
    + RuntimeEventProcessor
    + 'static
{
    /// Chain root hash used for transaction verification. Generated from a
    /// [schema](sov_rollup_interface::sov_universal_wallet::schema::Schema).
    const CHAIN_HASH: [u8; 32];

    /// GenesisConfig type.
    type GenesisConfig: Clone + Send + Sync + GenesisParams;

    /// GenesisInput type.
    type GenesisInput: std::fmt::Debug + Clone + Send + Sync;

    /// ModuleExecutionConfiguration type.
    type ModuleExecutionConfig: ModuleExecutionConfig;

    /// Responsible for authenticating transactions.
    type Auth: TransactionAuthenticator<S>;

    /// Decodes serialized call message.
    fn decode_call(
        serialized_message: &[u8],
    ) -> Result<<Self as DispatchCall>::Decodable, io::Error> {
        decode_borsh_serialized_message::<<Self as DispatchCall>::Decodable>(serialized_message)
    }

    /// Default RPC methods and Axum router.
    fn endpoints(storage: crate::rest::ApiState<S>) -> NodeEndpoints;

    /// Reads genesis configs.
    fn genesis_config(input: &Self::GenesisInput) -> anyhow::Result<Self::GenesisConfig>;

    /// Gets the operating mode of the runtime (Zk or Optimistic).
    fn operating_mode(genesis: &Self::GenesisConfig) -> OperatingMode;

    /// Wraps [`TransactionAuthenticator::Input`] into a call message.
    fn wrap_call(
        auth_data: <Self::Auth as TransactionAuthenticator<S>>::Decodable,
    ) -> Self::Decodable;

    /// Gets the processing delay in milliseconds for a given transaction.
    /// Returns 0 if no delay is configured.
    ///
    /// Runtimes can override this method to provide custom delay logic
    /// based on the transaction content. The delay will be applied before
    /// transaction processing begins.
    fn get_transaction_delay_ms(&self, _call: &Self::Decodable) -> u64 {
        0
    }

    /// Gets the priority level of a transaction. Higher priority transactions are processed first in the sequencer when possible.
    /// Messages with equal priority are processed in the order they are received. If messages have a delay configured in [`Runtime::get_transaction_delay_ms`],
    /// they are considered to be "received" after the delay period has elapsed.
    // Returns a u32 so that the sequencer can represent priority as a u64 and have some reserved values that are greater than the maximum priority level of any transaction.
    fn get_transaction_priority(&self, _call: &FullyBakedTx) -> u32 {
        0
    }

    /// Returns a call message to set the oracle timestamp if the runtime supports it.
    fn maybe_set_oracle_timestamp(
        &self,
        _millis_since_epoch: i64,
    ) -> Option<<Self as DispatchCall>::Decodable> {
        None
    }

    /// Checks if a system transaction should be rejected based on the totality of its context. For example,
    /// timing oracle updates that weren't submitted by the preferred sequencer should be rejected.
    fn is_unauthorized_system_tx(
        &self,
        _call: &Self::Decodable,
        _context: &Context<S>,
        _state: &mut impl crate::TxState<S>,
    ) -> bool {
        false
    }

    /// Populates the pinned state cache for the given storage if supported
    fn populate_pinned_cache(_storage: &S::Storage) -> Option<PinnedCache> {
        None
    }

    /// Resolve CredentialId to address.
    fn resolve_address<ST: StateReader<User>>(
        &self,
        default_address: &S::Address,
        credential_id: &crate::CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, ST::Error>;
}

#[cfg(feature = "native")]
/// Decodes borsh serialized message.
pub fn decode_borsh_serialized_message<T: borsh::BorshDeserialize>(
    mut serialized_message: &[u8],
) -> Result<T, io::Error> {
    let res = T::deserialize(&mut serialized_message)?;

    if !serialized_message.is_empty() {
        return Err(io::Error::other(
            "the provided message contains dangling data",
        ));
    }

    Ok(res)
}

/// This trait has to be implemented by a runtime in order to be used in `StfBlueprint`.
///
/// The `TxHooks` implementation sets up a transaction context based on the height at which it is
/// to be executed.
#[cfg(not(feature = "native"))]
pub trait Runtime<S: Spec>:
    DispatchCall<Spec = S>
    + HasCapabilities<S>
    + HasKernel<S>
    + Genesis<Spec = S, Config = Self::GenesisConfig>
    + TxHooks<Spec = S>
    + BlockHooks<Spec = S>
    + Default
    + RuntimeEventProcessor
    + 'static
{
    /// Chain root hash used for transaction verification. Generated from a
    /// [schema](sov_rollup_interface::sov_universal_wallet::schema::Schema).
    const CHAIN_HASH: [u8; 32];

    /// `GenesisConfig` type.
    type GenesisConfig: Clone + Send + Sync;

    /// Responsible for authenticating transactions.
    type Auth: TransactionAuthenticator<S>;

    /// Gets the operating mode of the runtime (Zk or Optimistic).
    fn operating_mode(genesis: &Self::GenesisConfig) -> OperatingMode;

    /// Wraps [`TransactionAuthenticator::Input`] into a call message.
    fn wrap_call(
        auth_data: <Self::Auth as TransactionAuthenticator<S>>::Decodable,
    ) -> Self::Decodable;

    /// Checks if a system transaction should be rejected based on the totality of its context. For example,
    /// timing oracle updates that weren't submitted by the preferred sequencer should be rejected.
    fn is_unauthorized_system_tx(
        &self,
        _call: &Self::Decodable,
        _context: &Context<S>,
        _state: &mut impl crate::TxState<S>,
    ) -> bool {
        false
    }
}

/// The return type of [`Runtime::endpoints`].
#[cfg(feature = "native")]
pub struct NodeEndpoints {
    /// The [`axum::Router`] for the runtime's HTTP server.
    pub axum_router: axum::Router<()>,
    /// A [`jsonrpsee::RpcModule`] for the runtime's JSON-RPC server.
    pub jsonrpsee_module: jsonrpsee::RpcModule<()>,
    /// A list of optional background tasks that have been spawned for the endpoints' purposes.
    ///
    /// These will be joined upon node shutdown.
    pub background_handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

#[cfg(feature = "native")]
impl Default for NodeEndpoints {
    fn default() -> Self {
        Self {
            axum_router: Default::default(),
            jsonrpsee_module: jsonrpsee::RpcModule::new(()),
            background_handles: Vec::new(),
        }
    }
}

/// Helper function to get [`sov_universal_wallet::schema::Schema`] for the [`Runtime`]
pub fn get_runtime_schema<S: Spec, R: TransactionCallable + DispatchCall + 'static>(
) -> anyhow::Result<sov_universal_wallet::schema::Schema> {
    let schema = sov_universal_wallet::schema::Schema::of_rollup_types_with_chain_data::<
        crate::transaction::Transaction<R, S>,
        crate::transaction::UnsignedTransaction<R, S>,
        R::Decodable,
        S::Address,
    >(sov_universal_wallet::schema::ChainData {
        chain_id: sov_modules_macros::config_value!("CHAIN_ID"),
        chain_name: sov_modules_macros::config_value!("CHAIN_NAME").to_string(),
    })?;
    Ok(schema)
}

/// A chain hash override for a range of block heights.
///
/// This allows SDK consumers to define different chain hashes for different height ranges,
/// enabling non-breaking upgrades when the rollup schema changes (e.g., adding a new transaction type).
///
/// The range is `[start_height, end_height)` - start is inclusive, end is exclusive.
/// The `grace_period` allows the hash to remain valid for additional blocks after `end_height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ChainHashOverride {
    /// The start height (inclusive).
    pub start_height: u64,
    /// The end height (exclusive).
    pub end_height: u64,
    /// The chain hash to use for this range.
    pub chain_hash: [u8; 32],
    /// Number of blocks after `end_height` during which this hash is still accepted.
    /// During the grace period `[end_height, end_height + grace_period)`, both this
    /// hash and the next override's hash (or default) are valid.
    #[serde(default)]
    pub grace_period: u64,
}

impl ChainHashOverride {
    /// Returns true if the given height falls within this override's primary range.
    ///
    /// The range is `[start_height, end_height)` - start is inclusive, end is exclusive.
    pub const fn contains(&self, height: u64) -> bool {
        height >= self.start_height && height < self.end_height
    }

    /// Returns true if the given height falls within this override's grace period.
    ///
    /// The grace period is `[end_height, end_height + grace_period)`.
    pub const fn in_grace_period(&self, height: u64) -> bool {
        self.grace_period > 0
            && height >= self.end_height
            && height < self.end_height.saturating_add(self.grace_period)
    }
}

/// Resolved chain hashes for a given height.
///
/// Contains the primary hash and any additional hashes that are valid due to grace periods.
#[derive(Clone, Debug)]
pub struct ResolvedChainHashes {
    /// The primary chain hash for this height.
    pub primary: [u8; 32],
    /// Additional valid hashes due to grace periods from previous overrides.
    pub grace_period_hashes: Vec<[u8; 32]>,
}

impl ResolvedChainHashes {
    /// Returns an iterator over all valid hashes (primary first, then grace period hashes).
    pub fn iter(&self) -> impl Iterator<Item = &[u8; 32]> {
        std::iter::once(&self.primary).chain(self.grace_period_hashes.iter())
    }
}

/// Resolves all valid chain hashes for a given height by checking overrides.
///
/// Overrides must be contiguous and start at zero (validated at compile time by the
/// `config_value!` macro). If the height is beyond all overrides (including grace periods),
/// falls back to the default hash.
///
/// # Arguments
/// * `height` - The block height to resolve the chain hash for
/// * `overrides` - A slice of chain hash overrides (must be contiguous, starting at 0)
/// * `default_hash` - The default chain hash to use when no override matches
///
/// # Returns
/// A [`ResolvedChainHashes`] containing the primary hash and any grace period hashes.
pub fn resolve_chain_hashes(
    height: u64,
    overrides: &[ChainHashOverride],
    default_hash: [u8; 32],
) -> ResolvedChainHashes {
    let mut primary = default_hash;
    let mut grace_period_hashes = Vec::new();

    // Find the primary hash for this height
    for hash_override in overrides {
        if hash_override.contains(height) {
            primary = hash_override.chain_hash;
            break;
        }
    }

    // If no override contains the height, check if we're beyond all overrides
    if primary == default_hash {
        if let Some(last) = overrides.last() {
            if height < last.end_height.saturating_add(last.grace_period) {
                // We're in the last override's grace period, primary is default
                // but we need to check grace periods below
            }
        }
    }

    // Collect any grace period hashes from previous overrides
    for hash_override in overrides {
        if hash_override.in_grace_period(height) {
            // Don't add if it's already the primary
            if hash_override.chain_hash != primary {
                grace_period_hashes.push(hash_override.chain_hash);
            }
        }
    }

    ResolvedChainHashes {
        primary,
        grace_period_hashes,
    }
}
