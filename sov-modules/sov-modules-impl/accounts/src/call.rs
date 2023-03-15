use crate::Address;

use super::Accounts;

use anyhow::{anyhow, bail, ensure, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::CallResponse;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    CreateAccount,
    UpdatePublicKey(C::PublicKey),
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn create_account(&mut self, context: &C) -> Result<CallResponse> {
        todo!()
    }

    pub(crate) fn update_public_key(
        &mut self,
        new_pub_key: C::PublicKey,
        context: &C,
    ) -> Result<CallResponse> {
        anyhow::ensure!(
            self.accounts.get(&new_pub_key).is_none(),
            "New Public Key already exists"
        );

        let account = self.accounts.get_or_err(context.sender())?;
        self.accounts.set(&new_pub_key, account);

        Ok(CallResponse::default())
    }
}
