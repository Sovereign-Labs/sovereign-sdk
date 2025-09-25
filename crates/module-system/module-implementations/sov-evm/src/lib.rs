#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod account_storage_key;
mod call;
mod config;
mod db;
mod evm;
mod genesis;
mod hooks;
#[cfg(feature = "native")]
mod metrics;
mod sov_evm;
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

use alloy_primitives::U256;
use alloy_primitives::{Address, B256};
pub use authenticate::{
    authenticate, decode_evm_tx, Eip712Authenticator, EthereumAuthenticator, EvmAuthenticator,
    EvmAuthenticatorInput, SchemaProvider,
};
pub use revm::primitives::hardfork::SpecId;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::Amount;
use sov_modules_api::prelude::UnwrapInfallible as _;
use sov_modules_api::{
    AccessoryStateMap, AccessoryStateReader, AccessoryStateReaderAndWriter, AccessoryStateValue,
    Context, DaSpec, GenesisState, InfallibleStateAccessor, InfallibleStateReaderAndWriter, Module,
    ModuleId, ModuleInfo, Spec, StateAccessor, StateMap, StateReader, StateValue, StateVec,
    TxState,
};
use sov_state::codec::BcsCodec;
use sov_state::User;

use crate::account_storage_key::AccountStorageKey;
use crate::db::{DbAccount, EvmDb};
pub use crate::evm::primitive_types::TransactionSigned;
use crate::evm::primitive_types::{
    Block, PendingTransaction, Receipt, SealedBlock, TxSignedAndRecovered,
};

pub use conversions::convert_to_tx_signed;
pub use conversions::create_tx_env;
use revm::state::Bytecode;

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

    /// Storage for accounts.
    #[state]
    pub(crate) account_storage: StateMap<AccountStorageKey, U256, BcsCodec>,

    /// Mapping from code hash to code. Used for lazy-loading code into a contract account.
    #[state]
    pub(crate) code: StateMap<B256, Bytecode, BcsCodec>,

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
    pub receipts: AccessoryStateMap<u64, Receipt, BcsCodec>,

    /// Used only by the RPC: block_hash => block_number mapping.
    #[state]
    pub block_hashes: AccessoryStateMap<B256, u64, BcsCodec>,

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
}

impl<S: Spec> Module for Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Spec = S;

    type Config = EvmGenesisConfig;

    type CallMessage = CallMessage;

    type Event = ();

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
    ) -> anyhow::Result<()> {
        self.execute_call(msg, context, state)
    }
}

impl<S: Spec> Evm<S> {
    /// Get a EvmDb instance for the supplied state.
    pub fn get_db<'a, Ws: StateAccessor>(&self, state: &'a mut Ws) -> EvmDb<'a, Ws, S> {
        EvmDb::new(
            self.accounts.clone(),
            self.account_storage.clone(),
            self.code.clone(),
            state,
            self.bank_module.clone(),
        )
    }

    /// Access the Ethereum transaction receipt by number.
    pub fn receipt<Accessor: AccessoryStateReader>(
        &self,
        index: u64,
        state: &mut Accessor,
    ) -> Option<Receipt> {
        self.receipts.get(&index, state).unwrap_infallible()
    }

    /// Access the Ethereum transaction by number.
    pub fn transaction<Accessor: AccessoryStateReader>(
        &self,
        index: u64,
        state: &mut Accessor,
    ) -> Option<TxSignedAndRecovered> {
        self.transactions.get(&index, state).unwrap_infallible()
    }

    /// Access the Ethereum blocks.
    pub fn block_numbers<Accessor: AccessoryStateReaderAndWriter>(
        &self,
        state: &mut Accessor,
    ) -> RangeInclusive<u64> {
        self.block_numbers
            .get(state)
            .unwrap_infallible()
            .expect("Block numbers must be set")
    }

    /// Lookup an Ethereum account by address.
    pub fn get_account<Accessor: StateReader<User>>(
        &self,
        address: &Address,
        state: &mut Accessor,
    ) -> Result<Option<DbAccount>, Accessor::Error> {
        self.accounts.get(address, state)
    }

    /// Get the value from a storage slot.
    pub fn get_storage<Accessor: StateReader<User>>(
        &self,
        address: &Address,
        index: &U256,
        state: &mut Accessor,
    ) -> Result<Option<U256>, Accessor::Error> {
        self.account_storage.get(&(address, index), state)
    }

    /// Get the currently pending head block.
    pub fn pending_head<Accessor: AccessoryStateReader>(
        &self,
        state: &mut Accessor,
    ) -> Option<Block> {
        self.pending_head.get(state).unwrap_infallible()
    }

    /// Get the current head block.
    pub fn head<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<Block>, Accessor::Error> {
        self.head.get(state)
    }

    /// Get the current block env.
    pub fn block_env<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<Option<BlockEnv>, Accessor::Error> {
        self.block_env.get(state)
    }

    /// Get the Evm chain config.
    pub fn cfg<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<EvmRuntimeConfig, Accessor::Error> {
        let cfg = self
            .cfg
            .get(state)? // The config must be set at genesis.
            .expect("The impossible happened: EVM config is not set");
        Ok(cfg)
    }

    /// Get the Evm chain config.
    pub fn cfg_infallible<Accessor: InfallibleStateAccessor>(
        &self,
        state: &mut Accessor,
    ) -> EvmRuntimeConfig {
        self.cfg
            .get(state)
            .unwrap_infallible()
            // The config must be set at genesis.
            .expect("The impossible happened: EVM config is not set")
    }

    /// Access the pending Ethereum transactions.
    pub fn pending_transactions<Accessor: InfallibleStateReaderAndWriter<User>>(
        &self,
        state: &mut Accessor,
    ) -> Vec<PendingTransaction> {
        self.pending_transactions.collect_infallible(state)
    }

    /// Lookup the height of an Ethereum block based on the supplied hash.
    pub fn get_block_height_by_hash<Accessor: AccessoryStateReader>(
        &self,
        block_hash: &B256,
        state: &mut Accessor,
    ) -> Option<u64> {
        self.block_hashes.get(block_hash, state).unwrap_infallible()
    }

    /// Lookup the index of a Ethereum transaction based on the supplied hash.
    pub fn get_tx_index_by_hash<Accessor: AccessoryStateReader>(
        &self,
        tx_hash: &B256,
        state: &mut Accessor,
    ) -> Option<u64> {
        self.transaction_hashes
            .get(tx_hash, state)
            .unwrap_infallible()
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
