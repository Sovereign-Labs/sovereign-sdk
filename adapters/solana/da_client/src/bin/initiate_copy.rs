use alloc::rc::Rc;
use std::str::FromStr;

use anchor_client::{Client, Cluster};
use anchor_lang::solana_program::sysvar::clock::Clock;
use clap::Parser;
use copy::{accounts as copy_accounts, instruction as copy_instruction, PREFIX};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::signer::keypair::read_keypair_file;
use solana_sdk::sysvar::SysvarId;
use solana_sdk::system_program;
extern crate alloc;

const DEFAULT_RPC_URL: &str = "http://localhost:8899";
const DEFAULT_WS_URL: &str = "ws://localhost:8900";

pub struct CopyClient {
    pub rpc_url: String,
    pub ws_url: String,
    pub signer: Keypair,
    pub copy_program: Pubkey,
    pub copy_pda: (Pubkey, u8),
    pub clock_account: Pubkey,
    pub system_program: Pubkey,
}

impl CopyClient {
    pub fn new(rpc_url: String, ws_url: String, signer: Keypair, copy_program: &str) -> Self {
        let copy_program_pubkey = Pubkey::from_str(copy_program).unwrap();
        let (copy_pda, bump) =
            Pubkey::find_program_address(&[PREFIX.as_bytes()], &copy_program_pubkey);

        CopyClient {
            rpc_url,
            ws_url,
            signer,
            copy_program: Pubkey::from_str(copy_program).unwrap(),
            copy_pda: (copy_pda, bump),
            clock_account: Clock::id(),
            system_program: system_program::id(),
        }
    }

    pub fn send_transaction(&self, source_account: &Pubkey) -> anyhow::Result<Signature> {
        let creator_pubkey = self.signer.pubkey();
        let c = Client::new(
            Cluster::Custom(self.rpc_url.clone(), self.ws_url.clone()),
            Rc::new(self.signer.insecure_clone()),
        );
        let prog = c.program(self.copy_program).unwrap();

        let signature = prog
            .request()
            .accounts(copy_accounts::CopyHash {
                creator: creator_pubkey,
                source_account: *source_account,
                copy_account: self.copy_pda.0,
                clock: self.clock_account,
                system_program: self.system_program,
            })
            .args(copy_instruction::CopyHash {
                bump: self.copy_pda.1,
            })
            .options(CommitmentConfig {
                commitment: CommitmentLevel::Processed,
            })
            .send()?;
        Ok(signature)
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long, required = true)]
    /// Path to the signer key
    signer: String,

    #[arg(long, required = true)]
    /// b58 encoded address for the on chain sovereign blockroot program
    copy_program: String,

    #[arg(long, required = true)]
    account_for_proof: String,

    #[arg(short, long, default_value_t=DEFAULT_RPC_URL.to_string())]
    /// URL for solana RPC
    rpc_url: String,

    #[arg(short, long, default_value_t=DEFAULT_WS_URL.to_string())]
    /// URL for solana Websocket
    ws_url: String,
}

fn main() {
    let cli = Cli::parse();

    // required parameters
    let signer = cli.signer;
    let copy_program = &cli.copy_program;
    let account_for_proof = Pubkey::from_str(&cli.account_for_proof).unwrap();

    // optional overrides
    let rpc_url = cli.rpc_url;
    let ws_url = cli.ws_url;

    let signer_keypair = read_keypair_file(signer).unwrap();

    let copy_client = CopyClient::new(rpc_url, ws_url, signer_keypair, copy_program);
    let sig = copy_client.send_transaction(&account_for_proof);
    println!("{:?}", sig);
    // println!("account_list");
}
