#[cfg(feature = "experimental")]
pub mod call;
#[cfg(feature = "experimental")]
pub mod evm;
#[cfg(feature = "experimental")]
pub mod genesis;
#[cfg(feature = "experimental")]
pub mod hooks;
#[cfg(feature = "native")]
#[cfg(feature = "experimental")]
pub mod query;
#[cfg(feature = "experimental")]
pub mod signer;
#[cfg(feature = "smart_contracts")]
pub mod smart_contracts;
#[cfg(feature = "experimental")]
#[cfg(test)]
mod tests;
#[cfg(feature = "experimental")]
pub use experimental::{AccountData, Evm, EvmConfig};
#[cfg(feature = "experimental")]
pub use revm::primitives::SpecId;

#[cfg(feature = "experimental")]
mod experimental {
    use std::collections::HashMap;

    use reth_primitives::{Address, H256};
    use revm::primitives::{SpecId, KECCAK_EMPTY, U256};
    use sov_modules_api::{Error, ModuleInfo};
    use sov_state::codec::{BcsCodec, JsonCodec};
    use sov_state::WorkingSet;

    use super::evm::db::EvmDb;
    use super::evm::transaction::BlockEnv;
    use super::evm::{DbAccount, EvmChainConfig};
    #[derive(Clone, Debug)]
    pub struct AccountData {
        pub address: Address,
        pub balance: U256,
        pub code_hash: H256,
        pub code: Vec<u8>,
        pub nonce: u64,
    }

    impl AccountData {
        pub fn empty_code() -> H256 {
            KECCAK_EMPTY
        }

        pub fn balance(balance: u64) -> U256 {
            U256::from(balance)
        }
    }

    #[derive(Clone, Debug)]
    pub struct EvmConfig {
        pub data: Vec<AccountData>,
        pub chain_id: u64,
        pub limit_contract_code_size: Option<usize>,
        pub spec: HashMap<u64, SpecId>,
        pub coinbase: Address,
        pub starting_base_fee: u64,
        pub block_gas_limit: u64,
        pub genesis_timestamp: u64,
        pub block_timestamp_delta: u64,
    }

    impl Default for EvmConfig {
        fn default() -> Self {
            Self {
                data: vec![],
                chain_id: 1,
                limit_contract_code_size: None,
                spec: vec![(0, SpecId::LATEST)].into_iter().collect(),
                coinbase: Address::zero(),
                starting_base_fee: reth_primitives::constants::MIN_PROTOCOL_BASE_FEE,
                block_gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
                block_timestamp_delta: reth_primitives::constants::SLOT_DURATION.as_secs(),
                genesis_timestamp: 0,
            }
        }
    }

    #[allow(dead_code)]
    #[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
    #[derive(ModuleInfo, Clone)]
    pub struct Evm<C: sov_modules_api::Context> {
        #[address]
        pub(crate) address: C::Address,

        #[state]
        pub(crate) accounts: sov_state::StateMap<Address, DbAccount, BcsCodec>,

        #[state]
        pub(crate) cfg: sov_state::StateValue<EvmChainConfig, BcsCodec>,

        #[state]
        pub(crate) pending_block: sov_state::StateValue<BlockEnv, BcsCodec>,

        #[state]
        pub(crate) head_number: sov_state::StateValue<u64>,

        // TODO JsonCodec: This is a workaround for https://github.com/bincode-org/bincode/issues/245 which affects all
        // binary serialization formats.
        // 1. Implement custom types for Block, Transaction etc.. with borsh derived.
        // 2. Remove JsonCodec.
        #[state]
        pub(crate) blocks: sov_state::AccessoryStateMap<u64, reth_rpc_types::Block, JsonCodec>,

        #[state]
        pub(crate) block_hashes:
            sov_state::AccessoryStateMap<reth_primitives::H256, u64, JsonCodec>,

        #[state]
        pub(crate) pending_transactions:
            sov_state::AccessoryStateVec<reth_rpc_types::Transaction, JsonCodec>,

        #[state]
        pub(crate) transactions: sov_state::AccessoryStateMap<
            reth_primitives::H256,
            reth_rpc_types::Transaction,
            JsonCodec,
        >,

        #[state]
        pub(crate) receipts: sov_state::AccessoryStateMap<
            reth_primitives::U256,
            reth_rpc_types::TransactionReceipt,
            JsonCodec,
        >,

        #[state]
        pub(crate) code:
            sov_state::AccessoryStateMap<reth_primitives::H256, reth_primitives::Bytes, BcsCodec>,
    }

    impl<C: sov_modules_api::Context> sov_modules_api::Module for Evm<C> {
        type Context = C;

        type Config = EvmConfig;

        type CallMessage = super::call::CallMessage;

        fn genesis(
            &self,
            config: &Self::Config,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<(), Error> {
            Ok(self.init_module(config, working_set)?)
        }

        fn call(
            &self,
            msg: Self::CallMessage,
            context: &Self::Context,
            working_set: &mut WorkingSet<C::Storage>,
        ) -> Result<sov_modules_api::CallResponse, Error> {
            Ok(self.execute_call(msg.tx, context, working_set)?)
        }
    }

    impl<C: sov_modules_api::Context> Evm<C> {
        pub(crate) fn get_db<'a>(
            &self,
            working_set: &'a mut WorkingSet<C::Storage>,
        ) -> EvmDb<'a, C> {
            EvmDb::new(self.accounts.clone(), working_set)
        }
    }
}
