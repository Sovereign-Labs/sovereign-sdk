#![allow(unused_doc_comments)]
//! This module defines the core runtime of the demo rollup.
//! To add new functionality to your rollup:
//!   1. Add a new module dependency to your `Cargo.toml` file
//!   2. Add the module to the `Runtime` below
//!   3. Update `genesis.json` with any additional data required by your new module

pub mod address;
#[cfg(feature = "test-utils")]
mod test_utils;

pub use address::MultiAddressEvmSolana;

use sov_address::{EthereumAddress, FromVmAddress};
use sov_hyperlane_integration::{
    HyperlaneAddress, InterchainGasPaymaster, Mailbox as RawMailbox, MerkleTreeHook, Warp,
};
#[cfg(feature = "native")]
use sov_modules_api::macros::{expose_rpc, CliWallet};
use sov_modules_api::prelude::*;
use sov_modules_api::{Base58Address, DispatchCall, Event, Genesis, Hooks, MessageCodec, Spec};

/// The hyperlane mailbox type, parameterized with Warp as the recipient.
pub type Mailbox<S> = RawMailbox<S, Warp<S>>;

/// The runtime defines the logic of the rollup.
///
/// At a high level, the rollup node receives serialized "call messages" from the DA layer and
/// executes them as atomic transactions. Upon reception, the message is deserialized and forwarded
/// to an appropriate module.
///
/// The module-specific logic is implemented by module creators, but all the glue code responsible
/// for message deserialization/forwarding is handled by a rollup `runtime`.
///
/// To define the runtime, we need to specify all the modules supported by our rollup (see the
/// `Runtime` struct below)
///
/// The `Runtime` defines:
/// - how the rollup modules are wired up together.
/// - how the state of the rollup is initialized.
/// - how messages are dispatched to appropriate modules.
///
/// Runtime lifecycle:
///
/// 1. Initialization:
///     When a rollup is deployed for the first time, it needs to set its genesis state.
///     The `#[derive(Genesis)]` macro will generate a `Runtime::genesis(config)` method which returns
///     `Storage` with the initialized state.
///
/// 2. Calls:
///     The `Module` interface defines a `call` method which accepts a module-defined type and triggers
///     the specific `module logic.` In general, the point of a call is to change the module state,
///     but if the call throws an error, no state is updated (the transaction is reverted).
///
/// `#[derive(MessageCodec)]` adds deserialization capabilities to the `Runtime` (by implementing the
/// `decode_call` method).
/// `Runtime::decode_call` accepts a serialized call message and returns a type that implements the
/// `DispatchCall` trait.
///  The `DispatchCall` implementation (derived by a macro) forwards the message to the appropriate
///  module and executes its `call` method.
#[derive(Default, Clone, Genesis, Hooks, DispatchCall, Event, MessageCodec, RuntimeRestApi)]
#[cfg_attr(feature = "native", derive(CliWallet), expose_rpc)]
pub struct Runtime<S: Spec>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    /// The Bank module.
    pub bank: sov_bank::Bank<S>,
    /// The Sequencer Registry module.
    pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<S>,
    /// The Operator Incentives module.
    pub operator_incentives: sov_operator_incentives::OperatorIncentives<S>,
    /// The Attester Incentives module.
    pub attester_incentives: sov_attester_incentives::AttesterIncentives<S>,
    /// The Prover Incentives module.
    pub prover_incentives: sov_prover_incentives::ProverIncentives<S>,
    /// The Accounts module.
    pub accounts: sov_accounts::Accounts<S>,
    /// The uniqueness module.
    pub uniqueness: sov_uniqueness::Uniqueness<S>,
    /// The Chain state module.
    pub chain_state: sov_chain_state::ChainState<S>,
    /// The Blob storage module.
    pub blob_storage: sov_blob_storage::BlobStorage<S>,
    /// The Paymaster module.
    pub paymaster: sov_paymaster::Paymaster<S>,
    /// The Revenue Share module.
    pub revenue_share: sov_revenue_share::RevenueShare<S>,
    /// The Hyperlane Mailbox module.
    pub mailbox: Mailbox<S>,
    /// The Interchain Gas Paymaster module.
    pub interchain_gas_paymaster: InterchainGasPaymaster<S>,
    /// The Merkle Tree Hook module.
    pub merkle_tree_hook: MerkleTreeHook<S>,
    /// The Hyperlane Warp module.
    pub warp: Warp<S>,
    #[cfg_attr(feature = "native", cli_skip)]
    /// The EVM module.
    pub evm: sov_evm::Evm<S>,
    /// A module used in benchmarks to generate a wide range of transaction access patterns.
    pub access_pattern: sov_test_modules::access_pattern::AccessPattern<S>,
    /// A module for synthetic load testing and state operations.
    pub synthetic_load: sov_synthetic_load::SyntheticLoad<S>,
}
