use sov_bank::{Amount, CallMessage, Coins, TokenId};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::Spec;

use super::{
    BankAccount, BankChangeLogEntry, BankMessageGenerator, BankTag, InternalMessageGenError,
    InternalMessageGenResult,
};
use crate::interface::{GeneratedMessage, GeneratorState, PickRandom};
use crate::state::TokenInfo;
use crate::{repeatedly, MessageOutcome};

#[derive(Debug, Clone, Arbitrary)]
enum InvalidMintReasons {
    /// The caller isn't authorized to mint the token
    CallerIsNotAuthorized,
    /// The token we're trying to mint doesn't exist
    TokenDoesNotExist,
    /// The mint amount would cause the balance to overflow
    MintAmountOverflows,
}

impl<S: Spec> BankMessageGenerator<S> {
    fn generate_invalid_mint_helper(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
        invalid_mint_reason: InvalidMintReasons,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        // We return a token ID along with its info and an arbitrary account that:
        // - Can mint it if the token exists and is mintable
        // - Is random in all the other cases
        let (token_id, token_info, acct) = {
            if let InvalidMintReasons::TokenDoesNotExist = invalid_mint_reason {
                // An arbitrary token ID will never exist
                let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
                (
                    TokenId::arbitrary(u)?,
                    TokenInfo {
                        total_supply: Amount::ZERO,
                    },
                    acct,
                )
            } else if let Some((_, acct)) =
                generator_state.get_random_existing_account_with_tag(BankTag::CanMint.into(), u)?
            {
                let token = *acct.can_mint().random_entry(u)?;
                let token_info = generator_state
                    .get_token(&token)
                    .expect("Token should exist. The generator state is corrupted.");
                (token, token_info, acct)
            } else {
                // If we can't get a valid account, we get a random token and a random account.
                let (token_id, token_info) = generator_state.get_random_token(u)?;
                let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
                (token_id, token_info, acct)
            }
        };

        let amount = {
            if let InvalidMintReasons::MintAmountOverflows = invalid_mint_reason {
                // Generate an amount large enough to overflow if possible
                let max_amount_without_overflow =
                    u128::MAX.saturating_sub(token_info.total_supply.0);

                if let Some(max_amount_without_overflow) =
                    max_amount_without_overflow.checked_add(1)
                {
                    u.int_in_range((max_amount_without_overflow + 1)..=u128::MAX)?
                } else {
                    // If we can't overflow, we try to generate another message.
                    return self.generate_invalid_mint(u, generator_state);
                }
            } else {
                u128::arbitrary(u)?
            }
        };

        let acct = if let InvalidMintReasons::CallerIsNotAuthorized = invalid_mint_reason {
            repeatedly!(
                let addr_and_acct = generator_state.get_or_generate(self.address_creation_rate, u)?;
                until: !addr_and_acct.1.can_mint().contains(&token_id),
                on_failure: return self.generate_invalid_mint(u, generator_state)
            );

            addr_and_acct.1
        } else {
            acct
        };

        let key = acct.private_key.clone();

        let message = CallMessage::Mint {
            coins: Coins {
                token_id,
                amount: Amount::new(amount),
            },
            mint_to_address: S::Address::arbitrary(u)?,
        };

        Ok(GeneratedMessage::new(
            message,
            key,
            MessageOutcome::Reverted,
        ))
    }

    /// A mint can be invalid because...
    ///  - The token doesn't exist
    ///  - The caller isn't authorized for the token
    ///  - The mint amount would cause the balance to overflow
    pub(super) fn generate_invalid_mint(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let invalid_mint_reason = InvalidMintReasons::arbitrary(u)?;
        self.generate_invalid_mint_helper(u, generator_state, invalid_mint_reason)
    }

    pub(super) fn generate_valid_mint(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let (_, acct) = generator_state
            .get_random_existing_account_with_tag(BankTag::CanMint.into(), u)?
            .ok_or(InternalMessageGenError::NoMintingAccounts)?;
        let token_id = *acct.can_mint().random_entry(u)?;
        let token_info = generator_state.get_token(&token_id).expect("Token exists");

        let key = acct.private_key.clone();
        let (recipient_addr, mut recipient_acct) =
            generator_state.get_or_generate(self.address_creation_rate, u)?;

        let amount_to_mint =
            Amount::new(u.int_in_range(0..=Amount::MAX.saturating_sub(token_info.total_supply).0)?);
        let recipient_balance = recipient_acct.increment_balance(Coins {
            token_id,
            amount: amount_to_mint,
        });

        let message = CallMessage::Mint {
            coins: Coins {
                token_id,
                amount: amount_to_mint,
            },
            mint_to_address: recipient_addr.clone(),
        };
        let mint_change =
            Self::update_state_with_mint(generator_state, token_id, token_info, amount_to_mint, u)?;

        generator_state.update_account(&recipient_addr, recipient_acct);

        Ok(GeneratedMessage::new(
            message,
            key,
            MessageOutcome::Successful {
                changes: vec![
                    BankChangeLogEntry::BalanceChanged {
                        address: recipient_addr.clone(),
                        coins: Coins {
                            token_id,
                            amount: Amount::new(recipient_balance),
                        },
                    },
                    mint_change,
                ],
            },
        ))
    }

    pub(super) fn update_state_with_mint(
        state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
        token_id: TokenId,
        mut old_token_info: TokenInfo,
        amount_minted: Amount,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> arbitrary::Result<BankChangeLogEntry<S>> {
        let new_supply = old_token_info
            .total_supply
            .checked_add(amount_minted)
            .expect("Token amount should have been checked for overflow");
        // If the supply is maxed, update our index to account for that, and remove the tag from affected accounts
        if new_supply == Amount::MAX {
            while let Some((addr, mut acct)) = state
                .get_random_existing_account_with_tag(BankTag::CanMintById(token_id).into(), u)?
            {
                acct.remove_can_mint(token_id);
                state.update_account(&addr, acct);
            }
        }

        old_token_info.total_supply = new_supply;
        state.update_token(token_id, old_token_info);

        Ok(BankChangeLogEntry::SupplyChanged {
            token_id,
            total_supply: new_supply,
        })
    }
}
