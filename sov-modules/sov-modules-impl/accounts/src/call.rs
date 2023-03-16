use crate::Account;
use crate::Accounts;
use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Signature;
use sov_modules_api::{Address, CallResponse, PublicKey};

pub const UPDATE_ACCOUNT_MSG: [u8; 32] = [0; 32];

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    CreateAccount,
    UpdatePublicKey(C::PublicKey, C::Signature),
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn create_account(&mut self, context: &C) -> Result<CallResponse> {
        self.exit_if_account_exist(context.sender())?;

        let default_address = context.sender().to_address();
        self.exit_if_address_exist(&default_address)?;

        let new_account = Account {
            addr: default_address,
            nonce: 0,
        };

        self.accounts.set(context.sender(), new_account);

        self.public_keys
            .set(&default_address, context.sender().clone());

        Ok(CallResponse::default())
    }

    pub(crate) fn update_public_key(
        &mut self,
        new_pub_key: C::PublicKey,
        signature: C::Signature,
        context: &C,
    ) -> Result<CallResponse> {
        self.exit_if_account_exist(&new_pub_key)?;

        let account = self.accounts.remove_or_err(context.sender())?;
        self.public_keys.remove_or_err(&account.addr)?;

        // Proof that the sender is in possession of the `new_pub_key`
        signature.verify(&new_pub_key, UPDATE_ACCOUNT_MSG)?;

        // Only the public key is updated, but account data remains the same.
        self.accounts.set(&new_pub_key, account);
        self.public_keys.set(&account.addr, new_pub_key);
        Ok(CallResponse::default())
    }

    fn exit_if_account_exist(&self, new_pub_key: &C::PublicKey) -> Result<()> {
        anyhow::ensure!(
            self.accounts.get(new_pub_key).is_none(),
            "New Public Key already exists"
        );
        Ok(())
    }

    fn exit_if_address_exist(&self, address: &Address) -> Result<()> {
        anyhow::ensure!(
            self.public_keys.get(address).is_none(),
            "Address already exists"
        );
        Ok(())
    }
}
