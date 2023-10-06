use anchor_client::{Client, Cluster};
use solana_sdk::{signer::keypair::read_keypair_file,pubkey::Pubkey,
                 signature::{Keypair, Signer},
                 system_instruction,
                 system_program,
                 transaction::Transaction,};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::clock::Clock;

use blockroot::accounts as blockroot_accounts;
use blockroot::instruction as blockroot_instruction;

use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{

};


use clap::Parser;
use alloc::rc::Rc;
extern crate alloc;

const DEFAULT_URL: &str = "http://127.0.0.1:8899";

const CLOCK_PROGRAM_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const BLOCKROOT_PROGRAM_ID: &str = "6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E";

fn create_account(
    client: &RpcClient,
    payer: &Keypair,
    new_account: &Keypair,
    space: u64,
) -> anyhow::Result<()> {
    let rent = client.get_minimum_balance_for_rent_exemption(space.try_into()?)?;
    let instr = system_instruction::create_account(
        &payer.pubkey(),
        &new_account.pubkey(),
        rent,
        space,
        &system_program::ID,
    );

    let blockhash = client.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &[instr],
        Some(&payer.pubkey()),
        &[payer, new_account],
        blockhash,
    );

    let _sig = client.send_and_confirm_transaction(&tx)?;

    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, required=true)]
    /// Path to the signer key
    signer: String,

    #[arg(short, long, default_value_t=DEFAULT_URL.to_string())]
    /// URL for solana RPC
    url: String,
}


fn main() {
    let args = Args::parse();
    let signer = args.signer;
    let url  = args.url;
    let kp = read_keypair_file(signer).unwrap();
    let c = Client::new(Cluster::Localnet, Rc::new(kp));
    let program_id: Pubkey = Pubkey::new(
        &bs58::decode(BLOCKROOT_PROGRAM_ID).into_vec().unwrap());
    let prog = c.program(program_id).unwrap();
    //
    // let clock_pubkey = Pubkey::new(&bs58::decode(CLOCK_PROGRAM_ID).into_vec().unwrap());
    // let signature = prog.request()
    //     .accounts(blockroot_accounts::Initialize {
    //         clock: clock_pubkey
    //     })
    //     .args(blockroot_instruction::Initialize {})
    //     .send();
}
