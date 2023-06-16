use std::convert::Infallible;

use revm::{
    db::{CacheDB, EmptyDB},
    primitives::{Account, AccountInfo, Bytecode, HashMap, B160, B256, U256},
    Database, DatabaseCommit,
};

pub(crate) struct EvmDb<'a> {
    pub(crate) db: &'a mut CacheDB<EmptyDB>,
}

impl<'a> Database for EvmDb<'a> {
    type Error = Infallible;

    fn basic(&mut self, address: B160) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: B160, index: U256) -> Result<U256, Self::Error> {
        self.db.storage(address, index)
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
        self.db.block_hash(number)
    }
}

impl<'a> DatabaseCommit for EvmDb<'a> {
    fn commit(&mut self, changes: HashMap<B160, Account>) {
        self.db.commit(changes)
    }
}
