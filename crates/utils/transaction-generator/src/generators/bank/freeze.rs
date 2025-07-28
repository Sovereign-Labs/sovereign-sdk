use sov_bank::{CallMessage, TokenId};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::{PrivateKey, PublicKey, Spec};

use super::{
    BankAccount, BankChangeLogEntry, BankMessageGenerator, BankTag, InternalMessageGenError,
    InternalMessageGenResult,
};
use crate::{repeatedly, GeneratedMessage, GeneratorState, MessageOutcome, PickRandom};

/// A token freeze can be invalid because...
///  - The token doesn't exist
///  - The caller isn't authorized for the token
///  - The token has already been frozen
#[derive(Debug, Clone, Arbitrary)]
enum InvalidFreezeReason {
    TokenDoesNotExist,
    CallerIsNotAuthorized,
    TokenAlreadyFrozen,
}

impl<S: Spec> BankMessageGenerator<S> {
    /// Generates a valid freeze. This can fail if there is no admin accounts.
    pub(crate) fn generate_valid_freeze(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let (_, acct) = generator_state
            .get_random_existing_account_with_tag(BankTag::CanMint.into(), u)?
            .ok_or(InternalMessageGenError::NoMintingAccounts)?;

        let token_id = *acct.can_mint().random_entry(u)?;

        let message = CallMessage::Freeze { token_id };

        let key = acct.private_key.clone();

        // State updates. We remove all the admins for the token.
        while let Some(mut acct) =
            generator_state.get_account_with_tag(BankTag::CanMintById(token_id).into())
        {
            acct.remove_can_mint(token_id);
            let addr = acct.private_key.pub_key().credential_id().into();
            generator_state.update_account(&addr, acct);
        }

        Ok(GeneratedMessage::new(
            message,
            key,
            MessageOutcome::Successful {
                changes: vec![BankChangeLogEntry::TokenFrozen { token_id }],
            },
        ))
    }

    fn generate_invalid_freeze_with_reason(
        &self,
        reason: InvalidFreezeReason,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        // We return a token ID along with its info and an arbitrary account that:
        // - Can freeze it if the token exists and the account is an admin
        // - Is random in all the other cases
        let (token_id, acct) = match reason {
            InvalidFreezeReason::TokenDoesNotExist => {
                // An arbitrary token ID will never exist
                let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
                (TokenId::arbitrary(u)?, acct)
            }
            InvalidFreezeReason::TokenAlreadyFrozen => {
                // We take a valid token that doesn't have any admins and an arbitrary account.
                repeatedly!(
                    let (token_id, _token_info) = generator_state.get_random_token(u)?;
                    until: generator_state.get_account_with_tag(BankTag::CanMintById(token_id).into()).is_none(),
                    on_failure: return self.generate_invalid_freeze(u, generator_state)
                );

                let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
                (token_id, acct)
            }
            InvalidFreezeReason::CallerIsNotAuthorized => {
                // We take a valid token and an arbitrary account that cannot mint it.
                let (token_id, _token_info) = generator_state.get_random_token(u)?;
                repeatedly!(
                    let addr_and_acct = generator_state.get_or_generate(self.address_creation_rate, u)?;
                    until: !addr_and_acct.1.can_mint().contains(&token_id),
                    on_failure: return self.generate_invalid_mint(u, generator_state)
                );

                (token_id, addr_and_acct.1)
            }
        };

        let message = CallMessage::Freeze { token_id };

        let key = acct.private_key.clone();

        Ok(GeneratedMessage::new(
            message,
            key,
            MessageOutcome::Reverted,
        ))
    }

    /// Generates an invalid freeze. This is always possible
    ///
    /// ## Error types
    /// If the specified token ID does not exist.
    ///
    /// If the `owner` is not a token admin.
    ///
    /// If the token has already been frozen.
    pub(crate) fn generate_invalid_freeze(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let reason = InvalidFreezeReason::arbitrary(u)?;
        self.generate_invalid_freeze_with_reason(reason, u, generator_state)
    }
}
