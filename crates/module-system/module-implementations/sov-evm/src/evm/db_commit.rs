use reth_primitives::revm_primitives::{Account, Address, HashMap};
use reth_primitives::U256;
use revm::DatabaseCommit;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, InfallibleStateAccessor, Spec};

use super::db::EvmDb;
use super::DbAccount;
use crate::to_rollup_address;

impl<Ws: InfallibleStateAccessor, S: Spec> DatabaseCommit for EvmDb<Ws, S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn commit(&mut self, mut changes: HashMap<Address, Account>) {
        // Cloned to release borrow
        let mut addresses: Vec<_> = changes.keys().cloned().collect();
        // Sort addresses to avoid non-determinism in ZK
        addresses.sort();

        for address in addresses {
            // Unwrap because we took key from map itself, so key exists by definition.
            let account = changes.remove(&address).unwrap();

            // TODO figure out what to do when account is destroyed.
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
            if account.is_selfdestructed() {
                todo!("Account destruction not supported")
            }

            let mut db_account = self
                .accounts
                .get(&address, &mut self.state)
                .unwrap_infallible()
                .unwrap_or_else(DbAccount::new);

            let mut account_info = account.info;
            let rollup_address: <S as Spec>::Address = to_rollup_address::<S>(address);

            self.bank_module
                .override_gas_balance(
                    // U256 can overflow u128
                    Amount::new(account_info.balance.try_into().unwrap()),
                    &rollup_address,
                    &mut self.state,
                )
                .unwrap_infallible();

            // Set the EVM account balance to 0 - as balances are stored in the bank module.
            account_info.balance = U256::ZERO;

            if let Some(ref code) = account_info.code {
                if !code.is_empty() {
                    // TODO: would be good to have a contains_key method on the StateMap that would be optimized, so we can check the hash before storing the code
                    self.code
                        .set(&account_info.code_hash, code.bytecode(), &mut self.state)
                        .unwrap_infallible();
                }
            }

            db_account.info = account_info;

            // Sort keys explicitly to avoid non-determinism.
            let mut account_storage_keys: Vec<_> = account.storage.keys().collect();
            account_storage_keys.sort();

            for key in account_storage_keys {
                // Unwrap because we took key from map itself, so key exists by definition.
                let value = account.storage.get(key).unwrap();
                let value = value.present_value();
                self.account_storage
                    .set(&(&address, key), &value, &mut self.state)
                    .unwrap_infallible();
            }

            self.accounts
                .set(&address, &db_account, &mut self.state)
                .unwrap_infallible();
        }
    }
}
