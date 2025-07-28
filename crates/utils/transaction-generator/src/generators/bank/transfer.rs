use sov_bank::{CallMessage, Coins};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::{Amount, Spec};

use super::{
    BankAccount, BankChangeLogEntry, BankMessageGenerator, BankTag, InternalMessageGenError,
    InternalMessageGenResult,
};
use crate::interface::{GeneratedMessage, GeneratorState};
use crate::{repeatedly, MessageOutcome, PickRandom};

#[derive(Arbitrary, Clone, Debug)]
enum InvalidTransferReasons {
    InvalidTokenID,
    AccountDoesNotExist,
    NotEnoughFunds,
}

impl<S: Spec> BankMessageGenerator<S> {
    fn generate_invalid_transfer_helper(
        &self,
        error_reason: InvalidTransferReasons,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let (coin, acct) = if let InvalidTransferReasons::InvalidTokenID = error_reason {
            // The probability that a random TokenID exists is the same as that of a hash collision
            let token_id = Arbitrary::arbitrary(u)?;

            // Sanity check, ensure the token doesn't exist
            assert!(generator_state.get_token(&token_id).is_none());

            let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
            (
                Coins {
                    amount: Amount::ZERO,
                    token_id,
                },
                acct,
            )
        } else if let Some(acct) = generator_state.get_account_with_tag(BankTag::HasBalance.into())
        {
            let coin = acct.balances.random_entry(u)?;

            (coin.clone(), acct)
        } else {
            let (token_id, _token_info) = generator_state.get_random_token(u)?;
            let (_, acct) = generator_state.get_or_generate(self.address_creation_rate, u)?;
            (
                Coins {
                    amount: Amount::ZERO,
                    token_id,
                },
                acct,
            )
        };

        let from_key = {
            if let InvalidTransferReasons::AccountDoesNotExist = error_reason {
                Arbitrary::arbitrary(u)?
            } else {
                acct.private_key
            }
        };

        let amount = if let InvalidTransferReasons::NotEnoughFunds = error_reason {
            // If the account has [`u64::MAX`] balance then try again to generate a different message
            if coin.amount == Amount::MAX {
                return self.generate_invalid_transfer(u, generator_state);
            }

            u.int_in_range(coin.amount.0 + 1..=u128::MAX)?
        } else {
            u.int_in_range(1..=u128::MAX)?
        };

        let to_address = S::Address::arbitrary(u)?;
        let message = CallMessage::Transfer {
            to: to_address,
            coins: Coins {
                token_id: coin.token_id,
                amount: Amount::new(amount),
            },
        };

        Ok(GeneratedMessage::new(
            message,
            from_key,
            MessageOutcome::Reverted,
        ))
    }

    /// A transfer can be invalid by:
    /// - transferring a token ID that doesn't exist
    /// - transferring from an account that doesn't exist
    /// - transferring from an account that doesn't hold the token
    /// - transferring more tokens than the account has.
    pub(crate) fn generate_invalid_transfer(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let invalid_reason = InvalidTransferReasons::arbitrary(u)?;
        self.generate_invalid_transfer_helper(invalid_reason, u, generator_state)
    }

    /// Generate a bank transfer message
    // we'll be able to use the trait methods directly for testing.
    #[allow(private_interfaces)]
    pub(crate) fn generate_valid_transfer(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let (from_addr, mut from_account) =
            Self::get_random_account_with_balance(generator_state, u)?;

        let from_key = from_account.private_key.clone();

        // Pick a random amount of a random token, and a random address to send it to
        let from_balance = from_account.pick_random_balance(u)?.unwrap_or_else(|| {
            panic!("We picked a non-empty account but {from_addr} had no tokens")
        });

        let token_id = from_balance.token_id;
        // A valid transfer has to come from an existing address
        // but it can go to a new one or an existing one
        let balance_to_send = u.int_in_range(1..=from_balance.amount.0)?;

        // Find a recipient who can receive that much balance
        repeatedly! {
            let (to_addr, to_account) = generator_state.get_or_generate(self.address_creation_rate, u)?;
            until: balance_to_send <= to_account.receivable_balance(token_id),
            on_failure: return Err(InternalMessageGenError::NoAccountsCanReceive(Coins {
                token_id,
                amount: balance_to_send.into(),
            })
        )}
        let mut to_account = to_account;

        // Construct the call message
        let coins_to_send = Coins {
            amount: balance_to_send.into(),
            token_id,
        };
        let msg = CallMessage::Transfer {
            to: to_addr.clone(),
            coins: coins_to_send.clone(),
        };

        // If this is a self-transfer, then it should be a no-op. No balances need to change,
        // and there are no state changes to cache.
        if from_addr == to_addr {
            return Ok(GeneratedMessage::new(
                msg,
                from_key,
                MessageOutcome::Successful { changes: vec![] },
            ));
        }

        // Otherwise, account for the balance changes
        let receiver_balance = to_account.increment_balance(coins_to_send.clone());
        let remaining_from_balance = from_account.decrement_balance(coins_to_send.clone());
        // Save back the modified state and compute the changelog
        generator_state.update_account(&to_addr, to_account);
        generator_state.update_account(&from_addr, from_account);

        // Finally, return the generated message
        Ok(GeneratedMessage::new(
            msg,
            from_key,
            MessageOutcome::Successful {
                changes: vec![
                    BankChangeLogEntry::BalanceChanged {
                        address: from_addr.clone(),
                        coins: Coins {
                            token_id,
                            amount: remaining_from_balance.into(),
                        },
                    },
                    BankChangeLogEntry::BalanceChanged {
                        address: to_addr,
                        coins: Coins {
                            token_id,
                            amount: receiver_balance.into(),
                        },
                    },
                ],
            },
        ))
    }

    fn get_random_account_with_balance(
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
        u: &mut arbitrary::Unstructured<'_>,
    ) -> InternalMessageGenResult<(S::Address, BankAccount<S>)> {
        generator_state
            .get_random_existing_account_with_tag(BankTag::HasBalance.into(), u)?
            .ok_or(InternalMessageGenError::NoAccountWithBalance)
    }
}
