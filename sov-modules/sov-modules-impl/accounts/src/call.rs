use crate::Account;
use crate::Accounts;
use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::Signature;
use sov_modules_api::{Address, CallResponse, PublicKey};

pub const UPDATE_ACCOUNT_MSG: [u8; 32] = [1; 32];

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    // Updates a PublicKey for the corresponding Account.
    // The sender must be in possession of the new PublicKey.
    UpdatePublicKey(C::PublicKey, C::Signature),
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn update_public_key(
        &mut self,
        new_pub_key: C::PublicKey,
        signature: C::Signature,
        context: &C,
    ) -> Result<CallResponse> {
        self.exit_if_account_exists(&new_pub_key)?;

        let account = self.accounts.remove_or_err(context.sender())?;

        // Sanity check.
        anyhow::ensure!(
            // This is guaranteed to be true.
            self.public_keys.get(&account.addr).is_some(),
            "Missing PublicKey"
        );

        // Proof that the sender is in possession of the `new_pub_key`.
        signature.verify(&new_pub_key, UPDATE_ACCOUNT_MSG)?;

        // Update the public key (account data remains the same).
        self.accounts.set(&new_pub_key, account);
        self.public_keys.set(&account.addr, new_pub_key);
        Ok(CallResponse::default())
    }

    fn exit_if_account_exists(&self, new_pub_key: &C::PublicKey) -> Result<()> {
        anyhow::ensure!(
            self.accounts.get(new_pub_key).is_none(),
            "New PublicKey already exists"
        );
        Ok(())
    }

    fn exit_if_address_exists(&self, address: &Address) -> Result<()> {
        anyhow::ensure!(
            self.public_keys.get(address).is_none(),
            "Address already exists"
        );
        Ok(())
    }
}
