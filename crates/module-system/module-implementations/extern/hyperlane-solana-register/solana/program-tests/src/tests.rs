use borsh::BorshSerialize;
use hyperlane_core::{
    accumulator::incremental::IncrementalMerkle, utils::hex_or_base58_or_bech32_to_h256, Encode,
    HyperlaneMessage
};
use hyperlane_sealevel_mailbox::{
    accounts::{DispatchedMessage, DispatchedMessageAccount, Outbox, OutboxAccount},
    mailbox_dispatched_message_pda_seeds, mailbox_message_dispatch_authority_pda_seeds,
};
use hyperlane_solana_sovereign_register::{
    HyperlaneRegisterInstruction, RegisterError, RegisterMessage,
};
use hyperlane_test_utils::{initialize_mailbox, mailbox_id, process_instruction};
use solana_program_test::BanksClient;
use solana_sdk::{
    instruction::{AccountMeta, InstructionError},
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    system_program,
    transaction::TransactionError,
};

use crate::setup::{init_env, protocol_fee_config, LOCAL_DOMAIN, MAX_PROTOCOL_FEE, REMOTE_DOMAIN};

#[tokio::test]
async fn test_register_message_dispatch() {
    let mailbox_program = mailbox_id();
    let (mut banks_client, payer) = init_env().await;
    let protocol_fee_config = protocol_fee_config();

    let mailbox_accounts = initialize_mailbox(
        &mut banks_client,
        &mailbox_program,
        &payer,
        LOCAL_DOMAIN,
        MAX_PROTOCOL_FEE,
        protocol_fee_config.clone(),
    )
    .await
    .unwrap();

    let register_program = hyperlane_solana_sovereign_register::id();
    let embedded_user = Pubkey::new_unique();
    let message = RegisterMessage {
        destination: REMOTE_DOMAIN,
        embedded_user,
        recipient: "0x54b0b39fd02198dfaf116360668610d2a6c28833ed646a589cc54435c80f648d".to_string(),
    };

    let unique_message_account_keypair = Keypair::new();
    let (dispatch_authority_key, _expected_dispatch_authority_bump) = get_dispatch_authority();
    let (dispatched_message_account_key, _dispatched_message_bump) = Pubkey::find_program_address(
        mailbox_dispatched_message_pda_seeds!(&unique_message_account_keypair.pubkey()),
        &mailbox_accounts.program,
    );

    let accounts = vec![
        // 0. `[executable]` The Mailbox program.
        // And now the accounts expected by the Mailbox's OutboxDispatch instruction:
        // 1. `[writeable]` Outbox PDA.
        // 2. `[]` This program's dispatch authority.
        // 3. `[executable]` System program.
        // 4. `[executable]` SPL Noop program.
        // 5. `[signer]` Payer.
        // 6. `[signer]` Unique message account.
        // 7. `[writeable]` Dispatched message PDA. An empty message PDA relating to the seeds
        //    `mailbox_dispatched_message_pda_seeds` where the message contents will be stored.
        AccountMeta::new_readonly(mailbox_accounts.program, false),
        AccountMeta::new(mailbox_accounts.outbox, false),
        AccountMeta::new_readonly(dispatch_authority_key, false),
        AccountMeta::new_readonly(system_program::id(), false),
        AccountMeta::new_readonly(spl_noop::id(), false),
        AccountMeta::new(payer.pubkey(), true),
        AccountMeta::new(unique_message_account_keypair.pubkey(), true),
        AccountMeta::new(dispatched_message_account_key, false),
    ];
    let instruction = solana_sdk::instruction::Instruction {
        program_id: register_program,
        accounts,
        data: HyperlaneRegisterInstruction::SendRegister(message)
            .try_to_vec()
            .unwrap(),
    };

    let signature = process_instruction(
        &mut banks_client,
        instruction,
        &payer,
        &[&payer, &unique_message_account_keypair],
    )
    .await
    .unwrap();

    let mut expected_body = Vec::new();
    expected_body.extend_from_slice(&payer.pubkey().to_bytes());
    expected_body.extend_from_slice(&embedded_user.to_bytes());
    let expected_message = HyperlaneMessage {
        version: 3,
        nonce: 0,
        origin: LOCAL_DOMAIN,
        // The sender should be the program ID because its dispatch authority signed
        sender: register_program.to_bytes().into(),
        destination: REMOTE_DOMAIN,
        recipient: hex_or_base58_or_bech32_to_h256(
            "0x54b0b39fd02198dfaf116360668610d2a6c28833ed646a589cc54435c80f648d",
        )
        .unwrap(),
        body: expected_body,
    };

    assert_dispatched_message(
        &mut banks_client,
        signature,
        unique_message_account_keypair.pubkey(),
        dispatched_message_account_key,
        &expected_message,
    )
    .await;

    let mut expected_tree = IncrementalMerkle::default();
    expected_tree.ingest(expected_message.id());

    assert_outbox(
        &mut banks_client,
        mailbox_accounts.outbox,
        Outbox {
            local_domain: LOCAL_DOMAIN,
            outbox_bump_seed: mailbox_accounts.outbox_bump_seed,
            owner: Some(payer.pubkey()),
            tree: expected_tree.clone(),
            max_protocol_fee: MAX_PROTOCOL_FEE,
            protocol_fee: protocol_fee_config,
        },
    )
    .await;
}

#[tokio::test]
async fn test_fails_for_invalid_recipient() {
    let mailbox_program = mailbox_id();
    let (mut banks_client, payer) = init_env().await;
    let protocol_fee_config = protocol_fee_config();

    let mailbox_accounts = initialize_mailbox(
        &mut banks_client,
        &mailbox_program,
        &payer,
        LOCAL_DOMAIN,
        MAX_PROTOCOL_FEE,
        protocol_fee_config.clone(),
    )
    .await
    .unwrap();

    let register_program = hyperlane_solana_sovereign_register::id();
    let embedded_user = Pubkey::new_unique();
    let message = RegisterMessage {
        destination: REMOTE_DOMAIN,
        embedded_user,
        recipient: "abc123".to_string(),
    };

    let unique_message_account_keypair = Keypair::new();
    let (dispatch_authority_key, _expected_dispatch_authority_bump) = get_dispatch_authority();
    let (dispatched_message_account_key, _dispatched_message_bump) = Pubkey::find_program_address(
        mailbox_dispatched_message_pda_seeds!(&unique_message_account_keypair.pubkey()),
        &mailbox_accounts.program,
    );

    let accounts = vec![
        // 0. `[executable]` The Mailbox program.
        // And now the accounts expected by the Mailbox's OutboxDispatch instruction:
        // 1. `[writeable]` Outbox PDA.
        // 2. `[]` This program's dispatch authority.
        // 3. `[executable]` System program.
        // 4. `[executable]` SPL Noop program.
        // 5. `[signer]` Payer.
        // 6. `[signer]` Unique message account.
        // 7. `[writeable]` Dispatched message PDA. An empty message PDA relating to the seeds
        //    `mailbox_dispatched_message_pda_seeds` where the message contents will be stored.
        AccountMeta::new_readonly(mailbox_accounts.program, false),
        AccountMeta::new(mailbox_accounts.outbox, false),
        AccountMeta::new_readonly(dispatch_authority_key, false),
        AccountMeta::new_readonly(system_program::id(), false),
        AccountMeta::new_readonly(spl_noop::id(), false),
        AccountMeta::new(payer.pubkey(), true),
        AccountMeta::new(unique_message_account_keypair.pubkey(), true),
        AccountMeta::new(dispatched_message_account_key, false),
    ];
    let instruction = solana_sdk::instruction::Instruction {
        program_id: register_program,
        accounts,
        data: HyperlaneRegisterInstruction::SendRegister(message)
            .try_to_vec()
            .unwrap(),
    };

    let err = process_instruction(
        &mut banks_client,
        instruction,
        &payer,
        &[&payer, &unique_message_account_keypair],
    )
    .await
    .unwrap_err()
    .unwrap();

    assert_eq!(
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(RegisterError::InvalidRecipient as u32)
        ),
        err
    );
}

fn get_dispatch_authority() -> (Pubkey, u8) {
    let program_id = hyperlane_solana_sovereign_register::id();
    Pubkey::find_program_address(mailbox_message_dispatch_authority_pda_seeds!(), &program_id)
}

pub async fn assert_dispatched_message(
    banks_client: &mut BanksClient,
    dispatch_tx_signature: Signature,
    dispatch_unique_account_pubkey: Pubkey,
    dispatched_message_account_key: Pubkey,
    expected_message: &HyperlaneMessage,
) {
    // Get the slot of the tx
    let dispatch_tx_status = banks_client
        .get_transaction_status(dispatch_tx_signature)
        .await
        .unwrap()
        .unwrap();
    let dispatch_slot = dispatch_tx_status.slot;

    let dispatched_message_account = banks_client
        .get_account(dispatched_message_account_key)
        .await
        .unwrap()
        .unwrap();
    let dispatched_message =
        DispatchedMessageAccount::fetch(&mut &dispatched_message_account.data[..])
            .unwrap()
            .into_inner();
    assert_eq!(
        *dispatched_message,
        DispatchedMessage::new(
            expected_message.nonce,
            dispatch_slot,
            dispatch_unique_account_pubkey,
            expected_message.to_vec(),
        ),
    );
}

pub async fn assert_outbox(
    banks_client: &mut BanksClient,
    outbox_pubkey: Pubkey,
    expected_outbox: Outbox,
) {
    let outbox_account = banks_client
        .get_account(outbox_pubkey)
        .await
        .unwrap()
        .unwrap();

    let outbox = OutboxAccount::fetch(&mut &outbox_account.data[..])
        .unwrap()
        .into_inner();

    assert_eq!(*outbox, expected_outbox,);
}
