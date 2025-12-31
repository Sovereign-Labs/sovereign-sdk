use hyperlane_sealevel_mailbox::protocol_fee::ProtocolFee;
use hyperlane_test_utils::mailbox_id;
use solana_program_test::{processor, BanksClient, ProgramTest};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

pub const LOCAL_DOMAIN: u32 = 13775;
pub const REMOTE_DOMAIN: u32 = 69420;
pub const PROTOCOL_FEE: u64 = 1_000_000_000;
pub const MAX_PROTOCOL_FEE: u64 = 1_000_000_001;

pub fn protocol_fee_config() -> ProtocolFee {
    ProtocolFee {
        fee: PROTOCOL_FEE,
        beneficiary: Pubkey::new_unique(),
    }
}

pub async fn init_env() -> (BanksClient, Keypair) {
    let program_id = mailbox_id();
    let mut program_test = ProgramTest::new(
        "hyperlane_sealevel_mailbox",
        program_id,
        processor!(hyperlane_sealevel_mailbox::processor::process_instruction),
    );

    program_test.add_program("spl_noop", spl_noop::id(), processor!(spl_noop::noop));
    program_test.add_program(
        "hyperlane_solana_sovereign_register",
        hyperlane_solana_sovereign_register::id(),
        processor!(hyperlane_solana_sovereign_register::process_instruction),
    );
    // need test ISM otherwise `initialize_mailbox` hangs
    // this ISM just allows all messages, but we're only dispatching outbound messages
    // so I guess it's not used at all for our use case
    program_test.add_program(
        "hyperlane_sealevel_test_ism",
        hyperlane_sealevel_test_ism::id(),
        processor!(hyperlane_sealevel_test_ism::program::process_instruction),
    );

    let (banks_client, payer, _recent_blockhash) = program_test.start().await;

    (banks_client, payer)
}
