use sov_bank::{CallMessage, Coins, TokenId};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary};
use sov_modules_api::{Amount, Spec};

use super::{
    BankAccount, BankChangeLogEntry, BankMessageGenerator, BankTag, InternalMessageGenError,
    InternalMessageGenResult,
};
use crate::{GeneratedMessage, GeneratorState, MessageOutcome, PickRandom, TokenInfo};

#[derive(Debug, Clone, Arbitrary)]
enum InvalidBurnReason {
    TokenDoesNotExist,
    InsufficientTokenBalanceForOwner,
    TokenBalanceExceedsSupply,
}

impl<S: Spec> BankMessageGenerator<S> {
    /// Generates a valid burn. This can fail if there is no account holding a token balance.
    pub(crate) fn generate_valid_burn(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let Some((address, mut account)) =
            generator_state.get_random_existing_account_with_tag(BankTag::HasBalance.into(), u)?
        else {
            return Err(InternalMessageGenError::NoAccountWithBalance);
        };

        let random_coin = account
            .pick_random_balance(u)?
            .expect("Account should have at least one balance");

        let token_id = random_coin.token_id;
        let token_info = generator_state
            .get_token(&token_id)
            .expect("Token should exist. The generator state is corrupted.");

        let initial_balance = random_coin.amount;
        let amount_to_burn = Amount::new(u.int_in_range(0..=initial_balance.0)?);
        let amount_left = initial_balance.checked_sub(amount_to_burn).expect(
            "Initial balance should not underflow when burning. The generator state is corrupted.",
        );
        let new_supply = token_info.total_supply.checked_sub(amount_to_burn).expect("Total token supply should not underflow when burning. The generator state is corrupted.");

        let coins_to_burn = Coins {
            amount: amount_to_burn,
            token_id,
        };

        let call_message = CallMessage::Burn {
            coins: coins_to_burn.clone(),
        };

        let key = account.private_key.clone();

        // State updates
        account.decrement_balance(coins_to_burn.clone());
        generator_state.update_account(&address, account);
        generator_state.update_token(
            token_id,
            TokenInfo {
                total_supply: new_supply,
            },
        );

        Ok(GeneratedMessage {
            message: call_message,
            sender: key,
            outcome: MessageOutcome::Successful {
                changes: vec![
                    BankChangeLogEntry::BalanceChanged {
                        address,
                        coins: Coins {
                            amount: amount_left,
                            token_id,
                        },
                    },
                    BankChangeLogEntry::SupplyChanged {
                        token_id,
                        total_supply: new_supply,
                    },
                ],
            },
        })
    }

    fn generate_invalid_burn_with_reason(
        &self,
        reason: InvalidBurnReason,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        // We return a token ID along with its info and an arbitrary account that:
        // - Can burn it if the token exists and the account has some balance
        // - Is random in all the other cases
        let (coin, token_info, acct) = {
            if let InvalidBurnReason::TokenDoesNotExist = reason {
                // An arbitrary token ID will never exist
                let (_addr, acct) =
                    generator_state.get_or_generate(self.address_creation_rate, u)?;
                (
                    Coins {
                        token_id: TokenId::arbitrary(u)?,
                        amount: Amount::ZERO,
                    },
                    TokenInfo {
                        total_supply: Amount::ZERO,
                    },
                    acct,
                )
            } else if let Some((_addr, acct)) = generator_state
                .get_random_existing_account_with_tag(BankTag::HasBalance.into(), u)?
            {
                let coins = acct.balances.random_entry(u)?;
                let token_info = generator_state
                    .get_token(&coins.token_id)
                    .expect("Token should exist. The generator state is corrupted.");
                (coins.clone(), token_info, acct)
            } else {
                // If we can't get a valid account, we get a random token and a random account.
                let (token_id, token_info) = generator_state.get_random_token(u)?;
                let (_addr, acct) =
                    generator_state.get_or_generate(self.address_creation_rate, u)?;
                (
                    Coins {
                        token_id,
                        amount: Amount::ZERO,
                    },
                    token_info,
                    acct,
                )
            }
        };

        let balance_to_burn = if let InvalidBurnReason::TokenBalanceExceedsSupply = reason {
            // In that case we cannot burn more than the total supply so we try to generate another message
            if token_info.total_supply == u128::MAX {
                return self.generate_invalid_burn(u, generator_state);
            }

            u.int_in_range(token_info.total_supply.0 + 1..=u128::MAX)?
        } else if let InvalidBurnReason::InsufficientTokenBalanceForOwner = reason {
            // In that case we cannot burn more than the coin amount so we try to generate another message
            if coin.amount == u128::MAX {
                return self.generate_invalid_burn(u, generator_state);
            }

            u.int_in_range(coin.amount.0 + 1..=u128::MAX)?
        } else {
            u.int_in_range(0..=coin.amount.0)?
        };

        let call_message = CallMessage::Burn {
            coins: Coins {
                amount: Amount::new(balance_to_burn),
                token_id: coin.token_id,
            },
        };

        Ok(GeneratedMessage {
            message: call_message,
            sender: acct.private_key.clone(),
            outcome: MessageOutcome::Reverted,
        })
    }

    /// Generates an invalid burn transaction. This is always possible
    ///
    /// ## Error types
    /// If the specified token ID does not exist.
    ///
    /// If the `owner` has insufficient token balance to burn the requested amount.
    /// No tokens will be burned in this case.
    ///
    /// If the requested burn amount exceeds the token's total supply.
    /// No tokens will be burned in this case.
    pub(crate) fn generate_invalid_burn(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let reason = InvalidBurnReason::arbitrary(u)?;
        self.generate_invalid_burn_with_reason(reason, u, generator_state)
    }
}
