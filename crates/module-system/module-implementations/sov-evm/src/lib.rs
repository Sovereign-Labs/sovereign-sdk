#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod account_storage_key;
mod call;
mod config;
mod db;
mod evm;
#[cfg(feature = "native")]
pub mod execution_config;
mod genesis;
mod hooks;
#[cfg(feature = "native")]
mod metrics;
mod sov_evm;
mod sov_fee_and_gas_utils;
mod state_access;
use sov_rollup_interface::da::Time;
use sov_state::{Kernel, User};
use std::ops::RangeInclusive;

pub use call::*;
pub use config::*;
pub use evm::*;
pub use genesis::*;

#[cfg(feature = "native")]
mod rpc;

use revm::context::BlockEnv;
#[cfg(feature = "native")]
pub use rpc::*;

#[cfg(test)]
mod tests;

mod authenticate;
#[cfg(feature = "native")]
mod helpers;

use alloy_primitives::{Address, BlockHash, B256};
use alloy_primitives::{BlockNumber, U256};
#[cfg(feature = "native")]
pub use authenticate::build_request_preflight_auth;
pub use authenticate::{
    authenticate, decode_evm_tx, EthereumAuthenticator, EvmAuthenticator, EvmAuthenticatorInput,
};
pub use revm::primitives::hardfork::SpecId;
use serde::Serialize;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::Amount;
use sov_modules_api::{
    err_detail, AccessoryStateMap, AccessoryStateValue, Context, CoreModuleError, DaSpec,
    ErrorContext, ErrorDetail, GenesisState, Module, ModuleId, ModuleInfo, Spec, StateMap,
    StateReader, StateValue, StateVec, TxState, VersionReader,
};
use sov_state::codec::BcsCodec;

use crate::account_storage_key::AccountStorageKey;
use crate::db::DbAccount;
pub use crate::evm::primitive_types::BlockBody;
pub use crate::evm::primitive_types::TransactionSigned;
use crate::evm::primitive_types::{Block, PendingTransaction, TxSignedAndRecovered};

pub use crate::evm::primitive_types::{Receipt, SealedBlock, SyntheticBlockWithoutRootsAndBloom};

pub use conversions::convert_to_tx_signed;
pub use conversions::create_tx_env;
use revm::state::Bytecode;
use thiserror::Error;

#[cfg(feature = "native")]
/// Prepared inputs for running the STF transaction pipeline for runtime-parity gas estimation.
#[doc(hidden)]
pub struct PreparedRuntimeParityEstimate<S: Spec, Call> {
    /// The pre-exec working set pinned to the selected block context.
    pub pre_exec_working_set:
        sov_modules_api::PreExecWorkingSet<S, sov_modules_api::ApiStateAccessor<S>>,
    /// The authenticated transaction data plus the synthetic raw tx hash.
    pub authenticated_tx: sov_modules_api::transaction::AuthenticatedTransactionAndRawHash<S>,
    /// Authorization data used by the runtime transaction authorizer.
    pub auth_data: sov_modules_api::capabilities::AuthorizationData<S>,
    /// The decoded runtime call to dispatch through the STF helper.
    pub runtime_call: Call,
    /// The synthetic fully baked transaction, including sequencing metadata when needed.
    pub raw_tx: sov_modules_api::FullyBakedTx,
    /// The operating mode captured from the selected state.
    pub operating_mode: sov_modules_api::OperatingMode,
}

/// These values are associated with EIP-4844, which we do not support, but they must be set to a value other than None for CANCUN.
const EXCESS_BLOB_GAS: u64 = 0;
const BLOB_GAS_PRICE: u128 = 0;

/// The sov-evm module provides compatibility with the EVM.
#[allow(dead_code)]
#[derive(Clone, ModuleInfo)]
pub struct Evm<S: Spec> {
    /// The ID of the evm module.
    #[id]
    pub(crate) id: ModuleId,

    /// Mapping from account address to account state.
    #[state]
    pub(crate) accounts: StateMap<Address, DbAccount, BcsCodec>,

    /// The address allowed to make changes to the `cfg`.
    #[state]
    pub(crate) admin: StateValue<S::Address, BcsCodec>,

    /// Storage for accounts.
    #[state]
    pub(crate) account_storage: StateMap<AccountStorageKey, U256, BcsCodec>,

    /// Mapping from code hash to code. Used for lazy-loading code into a contract account.
    #[state]
    pub(crate) code: StateMap<B256, Bytecode, BcsCodec>,

    /// A set of addresses that are allowed to deploy new contracts
    #[state]
    pub(crate) contract_creation_allowlist: StateMap<Address, (), BcsCodec>,

    /// Mapping from block number to block hash. Used by EVM blockhash opcode. Contains only last 256 values.
    #[state]
    pub(crate) block_hashes: StateMap<BlockNumber, BlockHash, BcsCodec>,

    /// Chain configuration. This field is set in genesis.
    #[state]
    pub(crate) cfg: StateValue<EvmRuntimeConfig, BcsCodec>,

    /// Block environment used by the evm. This field is set in `begin_rollup_block_hook`.
    #[state]
    pub(crate) block_env: StateValue<BlockEnv, BcsCodec>,

    /// Transactions that will be added to the current block.
    /// A valid transaction is added to the vec on every call message.
    #[state]
    pub(crate) pending_transactions: StateVec<PendingTransaction, BcsCodec>,

    /// Head of the chain. The new head is set in `end_rollup_block_hook` but without the inclusion of the `state_root` field.
    /// The `state_root` is added in `begin_rollup_block_hook` of the next block because its calculation occurs after the `end_rollup_block_hook`.
    #[state]
    pub(crate) head: StateValue<Block, BcsCodec>,

    /// Used only by the RPC. This represents the head of the chain and is set in two distinct stages:
    ///  1. `end_rollup_block_hook`: the pending head is populated with data from pending_transactions.
    ///  2. `finalize_hook` the `root_hash` is populated.
    ///
    /// Since this value is not authenticated, it can be modified in the
    /// `finalize_hook` with the correct `state_root`.
    #[state]
    pub(crate) pending_head: AccessoryStateValue<Block, BcsCodec>,

    /// Tracks the blocks stored in the accessory state.
    #[state]
    pub block_numbers: AccessoryStateValue<RangeInclusive<u64>, BcsCodec>,

    /// Used only by the RPC: The vec is extended with `pending_head` in `finalize_hook`.
    #[state]
    pub blocks: AccessoryStateMap<u64, SealedBlock, BcsCodec>,

    /// Used only by the RPC: List of processed transactions.
    #[state]
    pub transactions: AccessoryStateMap<u64, TxSignedAndRecovered, BcsCodec>,

    /// Used only by the RPC: Receipts.
    #[state]
    pub receipts: AccessoryStateMap<u64, (Receipt, Time), BcsCodec>,

    /// Used only by the RPC: block_hash => block_number mapping.
    #[state]
    pub block_hash_to_number: AccessoryStateMap<B256, u64, BcsCodec>,

    /// Used only by the RPC: transaction_hash => transaction_index mapping.
    #[state]
    pub transaction_hashes: AccessoryStateMap<B256, u64, BcsCodec>,

    /// A reference to the Bank module.
    #[module]
    pub(crate) bank_module: sov_bank::Bank<S>,

    /// A reference to the Uniqueness module.
    #[module]
    pub(crate) uniqueness_module: sov_uniqueness::Uniqueness<S>,

    /// A reference to the ChainState module.
    #[module]
    pub(crate) chain_state_module: sov_chain_state::ChainState<S>,

    #[phantom]
    phantom: core::marker::PhantomData<S>,

    /// When true, the max fee check in the authenticator is disabled.
    /// This is set to true when an EvmRuntimeConfigUpdate with all fields None is received.
    #[state]
    pub(crate) disable_max_fee_check: StateValue<bool, BcsCodec>,

    /// Used only by the RPC: actual gas-token fee paid per tx index.
    #[state]
    pub receipt_fees: AccessoryStateMap<u64, Amount, BcsCodec>,
}

/// The top-level error type for all EVM module operations.
///
/// This enum wraps all specific error types that can occur during different
/// EVM operations, providing a unified error interface for the module.
#[derive(Debug, Error, Serialize)]
pub enum Error {
    /// An error occurred in a core module operation.
    #[error(transparent)]
    CoreModuleError(#[from] CoreModuleError),
}

impl ErrorDetail for Error {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        CoreModuleError::Generic(err).into()
    }
}

impl<S: Spec> Module for Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = S;

    type Config = EvmGenesisConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = ();

    type Error = Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        match msg {
            CallMessage::Call(tx) => Ok(self.execute_call(tx, context, state)?),
            CallMessage::UpdateRuntimeConfig(update) => {
                Ok(self.update_runtime_config(update, context, state)?)
            }
        }
    }
}

impl<S: Spec> Evm<S> {
    pub(crate) fn base_fee<Reader, E>(&self, state: &mut Reader) -> Result<u64, E>
    where
        Reader: VersionReader + StateReader<User, Error = E> + StateReader<Kernel, Error = E>,
    {
        let price = self
            .chain_state_module
            .base_fee_per_gas(state)?
            .expect("Base fee per gas must be set");
        Ok(price.as_ref()[0].0.try_into().expect(
            "EVM invariant violation: primary base fee does not fit in u64; this divergence is a bug",
        ))
    }

    /// Get the admin address.
    pub fn admin<Reader, E>(&self, state: &mut Reader) -> Result<S::Address, E>
    where
        Reader: StateReader<User, Error = E>,
    {
        let admin = self.admin.get(state)?;
        Ok(admin.expect("Admin must be set at genesis and cannot be removed"))
    }
}

pub(crate) fn to_rollup_address<S: Spec>(address: Address) -> S::Address
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    S::Address::from_vm_address(EthereumAddress::from(address))
}

pub(crate) fn to_rollup_balance(balance: U256) -> Amount {
    // Overflow is not possible here. The gas token’s supply_cap is bounded by u128::MAX,
    // which means no account can ever hold a balance greater than u128::MAX.
    let bank_amount = balance
        .try_into()
        .unwrap_or_else(|_| panic!("The impossible happened: Balance overflowed"));
    Amount::new(bank_amount)
}
