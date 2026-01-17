#[cfg(feature = "native")]
use alloy_eips::BlockId;
use alloy_primitives::Address;
use alloy_primitives::B256;
use alloy_primitives::U256;
use revm::context::BlockEnv;
use sov_modules_api::da::Time;
use sov_modules_api::prelude::UnwrapInfallible;
#[cfg(feature = "native")]
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::{
    AccessoryStateReader, AccessoryStateReaderAndWriter, InfallibleStateAccessor,
    InfallibleStateReaderAndWriter, Spec, StateReader,
};
#[cfg(feature = "native")]
use sov_rollup_interface::common::RollupHeight;
#[cfg(feature = "native")]
use sov_rpc_eth_types::EthApiError;
use sov_state::User;
use std::ops::RangeInclusive;

#[cfg(feature = "native")]
use crate::error::into_rpc_error;
#[cfg(feature = "native")]
use crate::SealedBlock;
use crate::{
    db::EvmDb,
    primitive_types::{Block, PendingTransaction, TxSignedAndRecovered},
    Evm, EvmRuntimeConfig, Receipt,
};

/// Non-trivial state reads
#[cfg(feature = "native")]
impl<S: Spec> Evm<S> {
    /// Gets block by number. Fails if prunned or in the future
    pub fn block<Accessor: AccessoryStateReaderAndWriter>(
        &self,
        number: u64,
        state: &mut Accessor,
    ) -> Result<SealedBlock, EthApiError> {
        let numbers = self.block_numbers(state);
        if number >= *numbers.end() {
            return Err(EthApiError::HeaderNotFound(BlockId::number(number)));
        }
        let Some(block) = self.blocks.get(&number, state)? else {
            return Err(EthApiError::PrunedHistoryUnavailable);
        };
        Ok(block)
    }

    /// Gets tx by idx. Fails if not found
    pub fn tx<Accessor: AccessoryStateReader>(
        &self,
        idx: u64,
        state: &mut Accessor,
    ) -> Result<TxSignedAndRecovered, EthApiError> {
        self.transaction(idx, state)
            .ok_or(EthApiError::UnknownTxIndex(idx))
    }

    /// Gets archival state before block `number`
    pub fn archival_state_pre_block(
        &self,
        number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ApiStateAccessor<S>, EthApiError> {
        let height = if number == 0 {
            RollupHeight::GENESIS
        } else {
            RollupHeight::new(number - 1)
        };
        match state.get_archival_state(height) {
            Ok(state) => Ok(state),
            Err(err) => Err(EthApiError::other(into_rpc_error(err))),
        }
    }
}

/// User state reads
impl<S: Spec> Evm<S> {
    /// Get a EvmDb instance for the supplied state.
    pub fn db<'a, Ws: StateReader<User>>(&self, state: &'a mut Ws) -> EvmDb<'a, Ws, S> {
        EvmDb::new(
            self.accounts.clone(),
            self.account_storage.clone(),
            self.code.clone(),
            self.block_hashes.clone(),
            state,
            self.bank_module.clone(),
        )
    }

    /// Get the value from a storage slot.
    pub fn storage<Accessor: StateReader<User>>(
        &self,
        address: &Address,
        index: &U256,
        state: &mut Accessor,
    ) -> Result<Option<U256>, Accessor::Error> {
        self.account_storage.get(&(address, index), state)
    }

    /// Get the current block env.
    pub fn block_env<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<BlockEnv, Accessor::Error> {
        let block_env = self.block_env.get(state)?;
        Ok(block_env.expect("block_env should be set in `begin_rollup_block_hook`"))
    }

    /// Get the Evm chain config.
    pub fn cfg<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<EvmRuntimeConfig, Accessor::Error> {
        let cfg = self.cfg.get(state)?;
        Ok(cfg.expect("EVM config must be set in genesis"))
    }

    /// Check if the max fee check is disabled.
    pub fn is_max_fee_check_disabled<Accessor: StateReader<User>>(
        &self,
        state: &mut Accessor,
    ) -> Result<bool, Accessor::Error> {
        Ok(self.disable_max_fee_check.get(state)?.unwrap_or(false))
    }
}

/// Accessory state reads
impl<S: Spec> Evm<S> {
    /// Access the pending Ethereum transactions.
    pub fn pending_transactions<Accessor: InfallibleStateReaderAndWriter<User>>(
        &self,
        state: &mut Accessor,
    ) -> Vec<PendingTransaction> {
        self.pending_transactions.collect_infallible(state)
    }

    /// Access the Ethereum transaction receipt by number.
    pub fn receipt<Accessor: AccessoryStateReader>(
        &self,
        index: u64,
        state: &mut Accessor,
    ) -> Option<(Receipt, Time)> {
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

    /// Lookup the index of a Ethereum transaction based on the supplied hash.
    pub fn tx_index<Accessor: AccessoryStateReader>(
        &self,
        tx_hash: &B256,
        state: &mut Accessor,
    ) -> Option<u64> {
        self.transaction_hashes
            .get(tx_hash, state)
            .unwrap_infallible()
    }

    /// Lookup the height of an Ethereum block based on the supplied hash.
    pub fn block_height<Accessor: AccessoryStateReader>(
        &self,
        block_hash: &B256,
        state: &mut Accessor,
    ) -> Option<u64> {
        self.block_hash_to_number
            .get(block_hash, state)
            .unwrap_infallible()
    }

    /// Get the currently pending head block.
    pub fn pending_head<Accessor: AccessoryStateReader>(
        &self,
        state: &mut Accessor,
    ) -> Option<Block> {
        self.pending_head.get(state).unwrap_infallible()
    }
}

/// Reads on accessory state set in genesis
impl<S: Spec> Evm<S> {
    /// Access the Ethereum blocks.
    pub fn block_numbers<Accessor: AccessoryStateReaderAndWriter>(
        &self,
        state: &mut Accessor,
    ) -> RangeInclusive<u64> {
        let block_numbers = self.block_numbers.get(state).unwrap_infallible();
        block_numbers.expect("Block numbers must be set in genesis")
    }

    /// Get the Evm chain config.
    pub fn cfg_infallible<Accessor: InfallibleStateAccessor>(
        &self,
        state: &mut Accessor,
    ) -> EvmRuntimeConfig {
        let cfg = self.cfg.get(state).unwrap_infallible();
        cfg.expect("EVM config must be set in genesis")
    }

    /// Get head block
    pub fn head<Accessor: InfallibleStateAccessor>(&self, state: &mut Accessor) -> Block {
        let head = self.head.get(state).unwrap_infallible();
        head.expect("Head is set in genesis and never deleted")
    }
}
