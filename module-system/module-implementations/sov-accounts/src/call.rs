use anyhow::{ensure, Result};
use sov_modules_api::{CallResponse, Context, Signature, WorkingSet};

use crate::Accounts;

/// To update the account's public key, the sender must sign this message as proof of possession of the new key.
pub const UPDATE_ACCOUNT_MSG: [u8; 32] = [1; 32];

/// Represents the available call messages for interacting with the sov-accounts module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema),
    derive(sov_modules_api::macros::CliWalletArg),
    schemars(
        bound = "C::PublicKey: ::schemars::JsonSchema, C::Signature: ::schemars::JsonSchema",
        rename = "CallMessage"
    )
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<C: Context> {
    /// Updates a public key for the corresponding Account.
    /// The sender must be in possession of the new key.
    UpdatePublicKey(
        /// The new public key
        C::PublicKey,
        /// A valid signature from the new public key
        C::Signature,
    ),
}

impl<C: Context> Accounts<C> {
    pub(crate) fn update_public_key(
        &self,
        new_pub_key: C::PublicKey,
        signature: C::Signature,
        context: &C,
        working_set: &mut WorkingSet<C>,
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
        signature.verify(&new_pub_key, &UPDATE_ACCOUNT_MSG)?;

        // Update the public key (account data remains the same).
        self.accounts.set(&new_pub_key, &account, working_set);
        self.public_keys
            .set(context.sender(), &new_pub_key, working_set);
        Ok(CallResponse::default())
    }

    fn exit_if_account_exists(
        &self,
        new_pub_key: &C::PublicKey,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.accounts.get(new_pub_key, working_set).is_none(),
            "New PublicKey already exists"
        );
        Ok(())
    }
}

#[cfg(all(feature = "arbitrary", feature = "native"))]
impl<'a, C> arbitrary::Arbitrary<'a> for CallMessage<C>
where
    C: Context,
    C::PrivateKey: arbitrary::Arbitrary<'a>,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        use sov_modules_api::PrivateKey;

        let secret = C::PrivateKey::arbitrary(u)?;
        let public = secret.pub_key();

        let payload_len = u.arbitrary_len::<u8>()?;
        let payload = u.bytes(payload_len)?;
        let signature = secret.sign(payload);

        Ok(Self::UpdatePublicKey(public, signature))
    }
}
