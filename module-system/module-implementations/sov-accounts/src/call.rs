use anyhow::{ensure, Result};
use sov_modules_api::{CallResponse, Signature};
use sov_state::WorkingSet;

use crate::Accounts;

pub const UPDATE_ACCOUNT_MSG: [u8; 32] = [1; 32];

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<C: sov_modules_api::Context> {
    // Updates a PublicKey for the corresponding Account.
    // The sender must be in possession of the new PublicKey.
    UpdatePublicKey(C::PublicKey, C::Signature),
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn update_public_key(
        &self,
        new_pub_key: C::PublicKey,
        signature: C::Signature,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.exit_if_account_exists(&new_pub_key, working_set)?;

        let pub_key = self.public_keys.get_or_err(context.sender(), working_set)?;

        let account = self.accounts.remove_or_err(&pub_key, working_set)?;
        // Sanity check
        ensure!(
            context.sender() == &account.addr,
            "Inconsistent account data"
        );

        // Proof that the sender is in possession of the `new_pub_key`.
        signature.verify(&new_pub_key, UPDATE_ACCOUNT_MSG)?;

        // Update the public key (account data remains the same).
        self.accounts.set(&new_pub_key, &account, working_set);
        self.public_keys
            .set(context.sender(), &new_pub_key, working_set);
        Ok(CallResponse::default())
    }

    fn exit_if_account_exists(
        &self,
        new_pub_key: &C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.accounts.get(new_pub_key, working_set).is_none(),
            "New PublicKey already exists"
        );
        Ok(())
    }
}
