use std::str::FromStr as _;

use borsh::{BorshDeserialize, BorshSerialize};
use hyperlane_core::H256;
use hyperlane_sealevel_mailbox::instruction::{Instruction as MailboxInstruction, OutboxDispatch};
use hyperlane_sealevel_mailbox::{mailbox_message_dispatch_authority_pda_seeds, spl_noop};
use solana_program::account_info::{next_account_info, AccountInfo};
use solana_program::entrypoint::ProgramResult;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program::{get_return_data, invoke_signed};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::{Pubkey, PUBKEY_BYTES};
use solana_program::{msg, pubkey};

solana_program::declare_id!("HX6EowhA5XwWj29iTFeqhprg1gUxHgv6RNUu4bRtUgob");
solana_program::entrypoint!(process_instruction);

#[cfg(feature = "debug-logs")]
macro_rules! debug_msg {
    ($($arg:tt)*) => {
        solana_program::msg!("[dbg] {}", format!($($arg)*))
    };
}

#[cfg(not(feature = "debug-logs"))]
macro_rules! debug_msg {
    ($($arg:tt)*) => {};
}

pub enum RegisterError {
    InvalidMailbox,
    InvalidRecipient,
}

impl From<RegisterError> for ProgramError {
    fn from(value: RegisterError) -> Self {
        ProgramError::Custom(value as u32)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct RegisterMessage {
    /// The destination domain id
    pub destination: u32,
    /// The pubkey of the embedded users wallet
    pub embedded_user: Pubkey,
    /// Recipient warp route in hex or base58 encoding
    pub recipient: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum HyperlaneRegisterInstruction {
    SendRegister(RegisterMessage),
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    debug_msg!("Processing instruction..");
    let instruction = HyperlaneRegisterInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    debug_msg!("Decoded instruction: {:?}", instruction);
    match instruction {
        HyperlaneRegisterInstruction::SendRegister(register_message) => {
            register(program_id, accounts, register_message)
        }
    }
}

pub fn register(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    register_message: RegisterMessage,
) -> ProgramResult {
    debug_msg!("Executing register instruction");

    let accounts_iter = &mut accounts.iter();
    let mailbox_program = next_account_info(accounts_iter)?;

    if *mailbox_program.key != trusted_mailbox() {
        debug_msg!(
            "Invalid mailbox program: {}, expected: {}",
            mailbox_program.key,
            trusted_mailbox()
        );
        return Err(RegisterError::InvalidMailbox.into());
    }

    // Account 2: Outbox PDA.
    let mailbox_outbox_account = next_account_info(accounts_iter)?;
    // Account 3: Dispatch authority.
    let dispatch_authority_account = next_account_info(accounts_iter)?;

    debug_msg!("Fetching dispatch authority");
    let (expected_dispatch_authority_key, expected_dispatch_authority_bump) =
        Pubkey::find_program_address(mailbox_message_dispatch_authority_pda_seeds!(), program_id);

    if dispatch_authority_account.key != &expected_dispatch_authority_key {
        debug_msg!(
            "Invalid dispatch authority: {}, expected: {}",
            dispatch_authority_account.key,
            expected_dispatch_authority_key
        );
        return Err(ProgramError::InvalidArgument);
    }

    // Account 4: System program.
    let system_program_account = next_account_info(accounts_iter)?;

    debug_msg!("Verifying system program");
    if system_program_account.key != &solana_program::system_program::id() {
        debug_msg!(
            "Invalid system program: {}, expected: {}",
            system_program_account.key,
            solana_program::system_program::id()
        );
        return Err(ProgramError::InvalidArgument);
    }

    // Account 5: SPL Noop program.
    let spl_noop = next_account_info(accounts_iter)?;

    debug_msg!("Verifying SPL Noop program");
    if spl_noop.key != &spl_noop::id() {
        debug_msg!(
            "Invalid SPL Noop program: {}, expected: {}",
            spl_noop.key,
            spl_noop::id()
        );
        return Err(ProgramError::InvalidArgument);
    }

    // Account 6: Payer.
    let payer_info = next_account_info(accounts_iter)?;
    // Account 7: Unique message account.
    // Defer to the checks in the Mailbox / IGP, no need to verify anything here.
    let unique_message_account = next_account_info(accounts_iter)?;
    // Account 8: Dispatched message PDA.
    // Similarly defer to the checks in the Mailbox to ensure account validity.
    let dispatched_message_account = next_account_info(accounts_iter)?;
    let accounts = vec![
        AccountMeta::new(*mailbox_outbox_account.key, false),
        AccountMeta::new_readonly(*dispatch_authority_account.key, true),
        AccountMeta::new_readonly(*system_program_account.key, false),
        AccountMeta::new_readonly(*spl_noop.key, false),
        AccountMeta::new(*payer_info.key, true),
        AccountMeta::new_readonly(*unique_message_account.key, true),
        AccountMeta::new(*dispatched_message_account.key, false),
    ];
    let account_infos = &[
        mailbox_outbox_account.clone(),
        dispatch_authority_account.clone(),
        system_program_account.clone(),
        spl_noop.clone(),
        payer_info.clone(),
        unique_message_account.clone(),
        dispatched_message_account.clone(),
    ];

    // TODO: IGP support?

    let dispatch_authority_seeds: &[&[u8]] =
        mailbox_message_dispatch_authority_pda_seeds!(expected_dispatch_authority_bump);
    let mut message_body = Vec::with_capacity(PUBKEY_BYTES * 2);
    message_body.extend_from_slice(&payer_info.key.to_bytes());
    message_body.extend_from_slice(&register_message.embedded_user.to_bytes());

    let recipient =
        H256::from_str(&register_message.recipient).map_err(|_| RegisterError::InvalidRecipient)?;

    debug_msg!(
        "Dispatching register message to domain {} for recipient {:?}",
        register_message.destination,
        recipient
    );
    let dispatch_instruction = MailboxInstruction::OutboxDispatch(OutboxDispatch {
        sender: *program_id,
        destination_domain: register_message.destination,
        recipient,
        message_body,
    });
    let mailbox_ixn = Instruction {
        program_id: *mailbox_program.key,
        data: dispatch_instruction.into_instruction_data()?,
        accounts,
    };

    debug_msg!("Invoking mailbox dispatch instruction");
    // Call the Mailbox program to dispatch the message.
    invoke_signed(&mailbox_ixn, account_infos, &[dispatch_authority_seeds])?;
    debug_msg!("Mailbox dispatch instruction invoked");

    // Parse the message ID from the return data from the prior dispatch.
    let (returning_program_id, returned_data) =
        get_return_data().ok_or(ProgramError::InvalidArgument)?;

    // The mailbox itself doesn't make any CPIs, but as a sanity check we confirm
    // that the return data is from the mailbox.
    if returning_program_id != *mailbox_program.key {
        return Err(ProgramError::InvalidArgument);
    }

    let message_id = H256::try_from_slice(&returned_data).expect("Mailbox returned invalid H256");
    msg!("message_id {:?}", message_id);

    Ok(())
}

fn trusted_mailbox() -> Pubkey {
    // First check for compile-time env var
    if let Some(mailbox_pubkey_str) = option_env!("HYPERLANE_MAILBOX_PUBKEY") {
        Pubkey::from_str(mailbox_pubkey_str)
            .expect("Invalid HYPERLANE_MAILBOX_PUBKEY environment variable")
    } else if cfg!(feature = "test-utils") {
        // Solana testing mailbox
        // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/e0ea8910c5911b88f8de5fc4b4940f2b17f52281/rust/sealevel/libraries/test-utils/src/lib.rs#L42
        pubkey!("692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9iu1")
    } else {
        // Solana testnet mailbox
        // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/e0ea8910c5911b88f8de5fc4b4940f2b17f52281/rust/sealevel/environments/testnet4/solanatestnet/core/program-ids.json#L2
        pubkey!("75HBBLae3ddeneJVrZeyrDfv6vb7SMC3aCpBucSXS5aR")
    }
}
