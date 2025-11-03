use alloy_primitives::{Address, U256};
use itertools::Itertools;
use revm::primitives::HashMap;
use revm::state::{Account, AccountInfo, EvmStorageSlot};
use revm_database_interface::TryDatabaseCommit;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::{Spec, StateAccessor};

use super::EvmDb;
use crate::db::{DbAccount, Error};
use crate::{to_rollup_address, to_rollup_balance};

impl<'a, Ws: StateAccessor, S: Spec> TryDatabaseCommit for EvmDb<'a, Ws, S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Error = Error<Ws>;

    fn try_commit(&mut self, changes: HashMap<Address, Account>) -> Result<(), Self::Error> {
        for (address, account) in changes
            .into_iter()
            // Sort addresses to avoid non-determinism in ZK
            .sorted_by_key(|(address, _)| *address)
        {
            self.commit_account(address, account)?;
        }

        Ok(())
    }
}

impl<'a, Ws: StateAccessor, S: Spec> EvmDb<'a, Ws, S>
where
    EvmDb<'a, Ws, S>: TryDatabaseCommit<Error = Error<Ws>>,
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn commit_account(
        &mut self,
        address: Address,
        account: Account,
    ) -> Result<(), <Self as TryDatabaseCommit>::Error> {
        // TODO figure out what to do when account is destroyed.
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/425
        if account.is_selfdestructed() {
            return Err(Error::SelfDestructUnsupported);
        }

        self.commit_storage(address, account.storage)?;

        let mut account = account.info;

        self.bank_module
            .override_gas_balance(
                to_rollup_balance(account.balance),
                &to_rollup_address::<S>(address),
                self.state,
            )
            .map_err(Error::State)?;
        // Set the EVM account balance to 0 - as balances are stored in the bank module.
        account.balance = U256::ZERO;

        self.commit_code(address, &account)?;
        self.accounts
            .set(&address, &DbAccount(account), self.state)
            .map_err(Error::State)?;

        Ok(())
    }

    fn commit_code(
        &mut self,
        address: Address,
        account: &AccountInfo,
    ) -> Result<(), <Self as TryDatabaseCommit>::Error> {
        if let Some(ref code) = account.code {
            if !code.is_empty() {
                if !self.cfg.contract_creation_policy.allows(&address) {
                    return Err(Error::ContractCreationDenied(address));
                }
                // TODO: would be good to have a contains_key method on the StateMap that would be optimized, so we can check the hash before storing the code
                self.code
                    .set(&account.code_hash, code, self.state)
                    .map_err(Error::State)?;
            }
        }
        Ok(())
    }

    fn commit_storage(
        &mut self,
        address: Address,
        storage: HashMap<U256, EvmStorageSlot>,
    ) -> Result<(), <Self as TryDatabaseCommit>::Error> {
        storage
            .into_iter()
            .sorted_by_key(|(key, _)| *key) // Sort keys explicitly to avoid non-determinism.
            .try_for_each(|(key, value)| {
                let value = value.present_value();
                self.account_storage
                    .set(&(&address, &key), &value, self.state)
                    .map_err(Error::State)
            })
    }
}
