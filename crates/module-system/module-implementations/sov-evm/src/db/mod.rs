use crate::{to_rollup_address, AccountStorageKey};
use alloy_primitives::{Address, B256, U256};
use derive_more::{Deref, Into};
use derive_new::new;
use revm::state::{AccountInfo, Bytecode};
use revm::{database_interface::DBErrorMarker, Database};
use serde::{Deserialize, Serialize};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::{BorshSerializedSize, TxState};
use sov_modules_api::{Spec, StateAccessor, StateMap, StateReader};
use sov_state::codec::BcsCodec;
use sov_state::SlotKey;
use sov_state::User;
use std::fmt::{self, Debug};

pub(crate) mod commit;
pub(crate) mod init;

#[derive(thiserror::Error, Deref)]
#[error(transparent)]
pub struct Error<Ws: StateAccessor>(<Ws as StateReader<User>>::Error);

impl<Ws: StateAccessor> Debug for Error<Ws> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<Ws: StateAccessor> DBErrorMarker for Error<Ws> {}

/// Stores information about an EVM account and a corresponding account state.
#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Default, Deref, Into)]
pub struct DbAccount(pub(crate) AccountInfo);

/// A queryable EVM database.
#[derive(new)]
pub struct EvmDb<'a, Ws, S: Spec> {
    pub(crate) accounts: StateMap<Address, DbAccount, BcsCodec>,
    pub(crate) account_storage: StateMap<AccountStorageKey, U256, BcsCodec>,
    pub(crate) code: StateMap<B256, Bytecode, BcsCodec>,
    pub(crate) state: &'a mut Ws,
    pub(crate) bank_module: sov_bank::Bank<S>,
}

impl<'a, Ws: TxState<S>, S: Spec> Database for EvmDb<'a, Ws, S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Error = Error<Ws>;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let maybe_account_info = self
            .accounts
            .get(&address, self.state)
            .map_err(Error)?
            .map(|acc| acc.0);

        let rollup_address: <S as Spec>::Address = to_rollup_address::<S>(address);

        let bank_balance = self
            .bank_module
            .get_balance_of(&rollup_address, sov_bank::config_gas_token_id(), self.state)
            .map_err(Error)?
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
        let key = SlotKey::from(code_hash.to_vec());

        if let Some(code) = self.state.get_cached::<CachedByteCode>(Some(key.clone())) {
            return Ok(code.code.clone());
        }

        // TODO move to new_raw_with_hash for better performance
        let bytecode = self
            .code
            .get(&code_hash, self.state)
            .map_err(Error)?
            .unwrap_or_default();

        self.state.put_cached::<CachedByteCode>(
            Some(key),
            CachedByteCode {
                code: bytecode.clone(),
            },
        );

        Ok(bytecode)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let storage_value: U256 = self
            .account_storage
            .get(&(&address, &index), self.state)
            .map_err(Error)?
            .unwrap_or_default();

        Ok(storage_value)
    }

    fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
        todo!("block_hash not yet implemented")
    }
}

pub(crate) struct CachedByteCode {
    pub code: Bytecode,
}

impl BorshSerializedSize for CachedByteCode {
    fn serialized_size(&self) -> usize {
        match &self.code {
            Bytecode::Eip7702(bytes) => bytes.raw.len(),
            Bytecode::LegacyAnalyzed(analyzed) => analyzed.bytecode().len(),
        }
    }
}
