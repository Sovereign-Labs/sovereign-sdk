#[cfg(feature = "experimental")]
pub mod call;
#[cfg(feature = "experimental")]
pub mod evm;
#[cfg(feature = "experimental")]
pub mod genesis;
#[cfg(feature = "native")]
#[cfg(feature = "experimental")]
pub mod query;
#[cfg(feature = "experimental")]
mod receipt;
#[cfg(feature = "experimental")]
#[cfg(test)]
mod tests;
#[cfg(feature = "experimental")]
pub use experimental::{AccountData, Evm, EvmConfig};
#[cfg(feature = "experimental")]
pub use receipt::TransactionReceipt;

#[cfg(feature = "experimental")]
mod experimental {
    use revm::primitives::{KECCAK_EMPTY, U256};
    use sov_modules_api::Error;
    use sov_modules_macros::ModuleInfo;
    use sov_state::WorkingSet;

    use super::evm::db::EvmDb;
    use super::evm::transaction::BlockEnv;
    use super::evm::{DbAccount, EthAddress};
    use crate::evm::{Bytes32, EvmTransaction};
    use crate::TransactionReceipt;

    #[derive(Clone)]
    pub struct AccountData {
        pub address: EthAddress,
        pub balance: Bytes32,
        pub code_hash: Bytes32,
        pub code: Vec<u8>,
        pub nonce: u64,
    }

    impl AccountData {
        pub fn empty_code() -> [u8; 32] {
            KECCAK_EMPTY.to_fixed_bytes()
        }

        pub fn balance(balance: u64) -> Bytes32 {
            U256::from(balance).to_le_bytes()
        }
    }

    #[derive(Clone)]
    pub struct EvmConfig {
        pub data: Vec<AccountData>,
    }

    #[allow(dead_code)]
    #[cfg_attr(feature = "native", derive(sov_modules_macros::ModuleCallJsonSchema))]
    #[derive(ModuleInfo, Clone)]
    pub struct Evm<C: sov_modules_api::Context> {
        #[address]
        pub(crate) address: C::Address,

        #[state]
        pub(crate) accounts: sov_state::StateMap<EthAddress, DbAccount>,

        #[state]
        pub(crate) block_env: sov_state::StateValue<BlockEnv>,

        #[state]
        pub(crate) transactions: sov_state::StateMap<Bytes32, EvmTransaction>,

        #[state]
        pub(crate) receipts: sov_state::StateMap<Bytes32, TransactionReceipt>,
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
