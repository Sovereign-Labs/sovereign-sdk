use sov_bank::{CallMessage, Coins};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary, Unstructured};
use sov_modules_api::{Amount, SafeString, SafeVec, SizedSafeString, Spec};

use super::{
    BankAccount, BankChangeLogEntry, BankMessageGenerator, BankTag, InternalMessageGenResult,
};
use crate::generators::bank::InternalMessageGenError;
use crate::interface::{GeneratedMessage, GeneratorState, Taggable};
use crate::state::TokenInfo;
use crate::{MessageOutcome, Percent};

const TOKEN_NAME: &str = "TEST_TOKEN_NAME";
/// To avoid collisions, we make sure token names have at least 15 characters.
/// Since there are at least 62 valid ascii characters for safe string, this gives a collision probability
/// of less than sqrt(62 ** 15), (i.e. 1 per few trillion txs) which is unlikely to ever cause problems
const MIN_TOKEN_NAME_LEN: usize = 15;

impl<S: Spec> BankMessageGenerator<S> {
    /// A create token is only invalid if the same account tries to reuse the same token name
    pub(crate) fn generate_invalid_create_token(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        let Some((_addr, acct)) = generator_state
            .get_random_existing_account_with_tag(BankTag::HasCreatedToken.into(), u)?
        else {
            return Err(InternalMessageGenError::NoAccountsHaveCreatedTokensYet);
        };

        Ok(GeneratedMessage::new(
            CallMessage::CreateToken {
                token_name: TOKEN_NAME.try_into().unwrap(),
                token_decimals: None,
                initial_balance: Arbitrary::arbitrary(u)?,
                mint_to_address: Arbitrary::arbitrary(u)?,
                admins: Arbitrary::arbitrary(u)?,
                supply_cap: None,
            },
            acct.private_key.clone(),
            MessageOutcome::Reverted,
        ))
    }

    /// Generate a valid create_token message with a custom address creation rate
    #[allow(private_interfaces)]
    pub(crate) fn generate_valid_create_token_with_creation_rate(
        &self,
        creation_rate: Percent,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        // Pick a creator address, and a token name. Compute the token ID
        let (creator_key, token_name, token_decimals, token_id) = {
            let (creator_address, mut creator_acct) =
                generator_state.get_or_generate(creation_rate, u)?;
            let creator_key = creator_acct.private_key.clone();
            creator_acct.add_tag(BankTag::HasCreatedToken);
            // Use the standard name for the first token of each account, then pick at random
            // This makes it easy to generate *invalid* messages, since we can just pick any account
            // that has already created a token and output another create_token from that account using the standard token name,
            // (recall that creating two tokens with the same name from the same account is not allowed)
            let token_name =
                if !generator_state.has_tag(&creator_address, BankTag::HasCreatedToken.into()) {
                    TOKEN_NAME.to_string().try_into().unwrap()
                } else {
                    arbitrary_safe_string(u, MIN_TOKEN_NAME_LEN)?
                };
            let token_decimals =
                if !generator_state.has_tag(&creator_address, BankTag::HasCreatedToken.into()) {
                    None
                } else {
                    Some(u.int_in_range(0..=39)?)
                };
            let new_token_id =
                sov_bank::get_token_id::<S>(token_name.as_str(), token_decimals, &creator_address);
            generator_state.update_account(&creator_address, creator_acct);
            (creator_key, token_name, token_decimals, new_token_id)
        };

        // Generate a list of minters, updating the state as necessary
        let minters = {
            let mut minters = SafeVec::new();
            for _ in 0..minters.max_size() {
                if bool::arbitrary(u)? {
                    break;
                }
                let (addr, mut acct) = generator_state.get_or_generate(creation_rate, u)?;
                acct.add_can_mint(token_id);
                generator_state.update_account(&addr, acct);
                minters
                    .try_push(addr)
                    .expect("Push must succed at least max_size times");
            }
            minters
        };

        // Generate a receiver and amount, updating the state as necessary
        let (recipient_address, amount) = {
            let (recipient_address, mut recipient_acct) =
                generator_state.get_or_generate(creation_rate, u)?;
            let amount = Arbitrary::arbitrary(u)?;
            recipient_acct.increment_balance(Coins { token_id, amount });
            generator_state.update_account(&recipient_address, recipient_acct);
            (recipient_address, amount)
        };
        let mint_event = Self::update_state_with_mint(
            generator_state,
            token_id,
            TokenInfo {
                total_supply: Amount::ZERO,
            },
            amount,
            u,
        )?;

        Ok(GeneratedMessage::new(
            CallMessage::CreateToken {
                token_name,
                token_decimals,
                initial_balance: amount,
                mint_to_address: recipient_address.clone(),
                admins: minters,
                supply_cap: None,
            },
            creator_key,
            MessageOutcome::Successful {
                changes: vec![
                    BankChangeLogEntry::BalanceChanged {
                        address: recipient_address,
                        coins: Coins { token_id, amount },
                    },
                    mint_event,
                ],
            },
        ))
    }

    /// Generate a valid create_token message. Same as [`Self::generate_valid_create_token_with_creation_rate`]
    /// with the default address creation rate.
    #[allow(private_interfaces)]
    pub(crate) fn generate_valid_create_token(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
    ) -> arbitrary::Result<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        self.generate_valid_create_token_with_creation_rate(
            self.address_creation_rate,
            u,
            generator_state,
        )
    }
}

fn arbitrary_safe_string(
    u: &mut Unstructured<'_>,
    min_len: usize,
) -> arbitrary::Result<SafeString> {
    let mut out = SizedSafeString::new();
    let target_len = u.int_in_range(min_len..=out.max_len() - 1)?;
    let mut i = 0;
    while out.len() < target_len && i < 10_000 {
        let next_char = u8::arbitrary(u)?;
        let _ = out.try_push(char::from(next_char));
        i += 1;
    }

    assert!(
        out.len() >= min_len,
        "Could not generate a valid safe string in 10_000 iters"
    );

    Ok(out)
}
