mod call;
mod genesis;
mod query;

use borsh::{BorshDeserialize, BorshSerialize};

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub struct Address {
    addr: [u8; 32],
}

impl Address {
    pub fn new<C: sov_modules_api::Context>(pub_key: &C::PublicKey) -> Self {
        todo!()
    }
}

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
struct Account {
    addr: Address,
    nonce: u64,
}

#[derive(ModuleInfo)]
pub struct Accounts<C: sov_modules_api::Context> {
    #[state]
    pub(crate) addresses: sov_state::StateMap<Address, C::PublicKey, C::Storage>,

    #[state]
    pub(crate) accounts: sov_state::StateMap<C::PublicKey, Account, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Accounts<C> {
    type Context = C;

    type CallMessage = sov_modules_api::NonInstantiable;

    type QueryMessage = query::QueryMessage<C>;
    // Add PreCheck

    fn genesis(&mut self) -> Result<(), Error> {
        Ok(self.init_module()?)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Self::Context,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        todo!()
    }

    #[cfg(feature = "native")]
    fn query(&self, msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
        match msg {
            query::QueryMessage::GetAccount(pub_key) => self.get_account(pub_key),
        };
        todo!()
    }
}
