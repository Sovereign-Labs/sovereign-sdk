use std::convert::Infallible;

use reth_primitives::revm_primitives::{AccountInfo, Address, Bytecode, B256, U256};
use reth_primitives::Bytes;
use revm::Database;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{InfallibleStateAccessor, Spec, StateMap};
use sov_state::codec::BcsCodec;

use super::DbAccount;
use crate::{to_rollup_address, AccountStorageKey};

/// A queryable EVM database.
pub struct EvmDb<Ws, S: Spec> {
    pub(crate) accounts: StateMap<Address, DbAccount, BcsCodec>,
    pub(crate) account_storage: StateMap<AccountStorageKey, U256, BcsCodec>,
    pub(crate) code: StateMap<B256, Bytes, BcsCodec>,
    pub(crate) state: Ws,
    pub(crate) bank_module: sov_bank::Bank<S>,
}

impl<Ws, S: Spec> EvmDb<Ws, S> {
    pub(crate) fn new(
        accounts: StateMap<Address, DbAccount, BcsCodec>,
        account_storage: StateMap<AccountStorageKey, U256, BcsCodec>,
        code: StateMap<B256, Bytes, BcsCodec>,
        state: Ws,
        bank_module: sov_bank::Bank<S>,
    ) -> Self {
        Self {
            accounts,
            account_storage,
            code,
            state,
            bank_module,
        }
    }
}

impl<Ws: InfallibleStateAccessor, S: Spec> Database for EvmDb<Ws, S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Error = Infallible;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let maybe_account_info = self
            .accounts
            .get(&address, &mut self.state)
            .unwrap_infallible()
            .map(|acc| acc.info);

        let rollup_address: <S as Spec>::Address = to_rollup_address::<S>(address);

        let bank_balance = self
            .bank_module
            .get_balance_of(
                &rollup_address,
                sov_bank::config_gas_token_id(),
                &mut self.state,
            )
            .unwrap_infallible()
            .unwrap_or_default();

        match maybe_account_info {
            Some(mut account_info) => {
                assert_eq!(
                    account_info.balance,
                    U256::ZERO,
                    "EVM balance is not zero - balance should be stored in the bank module instead"
                );

                account_info.balance = U256::from(bank_balance.0);
                Ok(Some(account_info))
            }
            // TODO: Here we generate a default account and set the balance from bank
            // and return that, however, not sure if EVM internally does any extra logic
            // when creating a new account. Create an issue to investigate.
            None => Ok(Some(AccountInfo::from_balance(U256::from(bank_balance.0)))),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // TODO move to new_raw_with_hash for better performance
        let bytecode = Bytecode::new_raw(
            self.code
                .get(&code_hash, &mut self.state)
                .unwrap_infallible()
                .unwrap_or_default(),
        );

        Ok(bytecode)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let storage_value: U256 = self
            .account_storage
            .get(&(&address, &index), &mut self.state)
            .unwrap_infallible()
            .unwrap_or_default();

        Ok(storage_value)
    }

    fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
        todo!("block_hash not yet implemented")
    }
}
