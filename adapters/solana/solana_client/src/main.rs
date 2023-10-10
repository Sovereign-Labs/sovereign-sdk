use anchor_client::{Client, Cluster};
use std::str::FromStr;
use solana_sdk::signature::EncodableKey;
use std::path::Path;
use solana_sdk::{signer::keypair::read_keypair_file,pubkey::Pubkey,
                 signature::{Keypair, Signer,Signature},
                 system_instruction,
                 system_program,
                 transaction::Transaction,
};
use solana_sdk::commitment_config::{CommitmentLevel,CommitmentConfig};

use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::clock::Clock;

use blockroot::accounts as blockroot_accounts;
use blockroot::instruction as blockroot_instruction;
use blockroot::{Chunk, get_chunks, ChunkAccumulator, PREFIX};

use solana_rpc_client::rpc_client::RpcClient;

use clap::{Parser,Subcommand,Args};
use alloc::rc::Rc;
extern crate alloc;

const DEFAULT_RPC_URL: &str = "http://localhost:8899";
const DEFAULT_WS_URL: &str = "ws://localhost:8900";

const CLOCK_PROGRAM_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const BLOCKROOT_PROGRAM_ID: &str = "6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E";
const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";

fn create_account(url: &str,
                  payer: &Keypair,
                  chunks_account: &str,
                  size: u64) -> anyhow::Result<Keypair> {
    let client = RpcClient::new(&url);
    let space = size;
    let new_account = Keypair::new();
    new_account.write_to_file(chunks_account).unwrap();

    let rent = client.get_minimum_balance_for_rent_exemption(space.try_into()?)?;
    let instr = system_instruction::create_account(
        &payer.pubkey(),
        &new_account.pubkey(),
        rent,
        space,
        &Pubkey::from_str(BLOCKROOT_PROGRAM_ID)?,
    );

    let blockhash = client.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &[instr],
        Some(&payer.pubkey()),
        &[payer, &new_account],
        blockhash,
    );
    client.send_and_confirm_transaction(&tx)?;
    Ok(new_account)
}

fn initialize_account(rpc_url: &str,
                      ws_url: &str,
                      payer: &Keypair,
                      chunks_keypair: &Keypair,
    ) -> anyhow::Result<Signature> {
    let creator_pubkey = payer.pubkey();
    let c = Client::new(Cluster::Custom(rpc_url.to_string(), ws_url.to_string()),
                        Rc::new(payer.insecure_clone()));
    let program_id: Pubkey = Pubkey::from_str(BLOCKROOT_PROGRAM_ID).unwrap();
    let prog = c.program(program_id).unwrap();
    let system_pubkey = Pubkey::new(&bs58::decode(SYSTEM_PROGRAM_ID).into_vec()?);

    let signature = prog.request()
        .accounts(blockroot_accounts::Initialize{
            creator: creator_pubkey,
            chunk_accumulator:chunks_keypair.pubkey(),
            system_program: system_pubkey,
        })
        .args(blockroot_instruction::Initialize {
        })
        .signer(chunks_keypair)
        .send()?;
    Ok(signature)
}

fn send_chunk_transaction(rpc_url: &str,
                          ws_url: &str,
                          payer: &Keypair,
                          chunks_keypair: &Keypair,
                          chunk: Chunk) -> anyhow::Result<Signature> {
    let creator_pubkey = payer.pubkey();
    let c = Client::new(Cluster::Custom(rpc_url.to_string(), ws_url.to_string()),
                        Rc::new(payer.insecure_clone()));
    let program_id: Pubkey = Pubkey::from_str(BLOCKROOT_PROGRAM_ID).unwrap();
    let prog = c.program(program_id).unwrap();

    let system_pubkey = Pubkey::new(&bs58::decode(SYSTEM_PROGRAM_ID).into_vec()?);
    let clock_pubkey = Pubkey::new(&bs58::decode(CLOCK_PROGRAM_ID).into_vec()?);
    let (blockroot_pda, bump) = Pubkey::find_program_address(&[PREFIX.as_bytes()], &program_id);
    let signature = prog.request()
        .accounts(blockroot_accounts::ProcessChunk{
            creator: creator_pubkey,
            chunk_accumulator:chunks_keypair.pubkey(),
            clock: clock_pubkey,
            blocks_root: blockroot_pda,
            system_program: system_pubkey,
        })
        .args(blockroot_instruction::ProcessChunk {
            bump,
            chunk
        })
        .options(CommitmentConfig {commitment:CommitmentLevel::Processed})
        .signer(chunks_keypair)
        .send()?;
    Ok(signature)
}

fn wipe_account(rpc_url: &str,
                      ws_url: &str,
                      payer: &Keypair,
                      chunks_keypair: &Keypair,
) -> anyhow::Result<Signature> {
    let creator_pubkey = payer.pubkey();
    let c = Client::new(Cluster::Custom(rpc_url.to_string(), ws_url.to_string()),
                        Rc::new(payer.insecure_clone()));
    let program_id: Pubkey = Pubkey::from_str(BLOCKROOT_PROGRAM_ID).unwrap();
    let prog = c.program(program_id).unwrap();
    let system_pubkey = Pubkey::new(&bs58::decode(SYSTEM_PROGRAM_ID).into_vec()?);

    let signature = prog.request()
        .accounts(blockroot_accounts::Clear{
            creator: creator_pubkey,
            chunk_accumulator:chunks_keypair.pubkey(),
            system_program: system_pubkey,
        })
        .args(blockroot_instruction::Clear {
            digest: None
        })
        .signer(chunks_keypair)
        .send()?;
    Ok(signature)
}

fn accumulate_chunks_get_root(chunks: Vec<Chunk>) -> Option<[u8;32]> {
    let raw_data_digest = chunks[0].digest;
    let mut ca = ChunkAccumulator::new();
    for c in chunks {
        ca.accumulate(c);
    }
    ca.get_merkle_root(&raw_data_digest)
}

fn create_large_account(
    rpc_url: &str,
    ws_url: &str,
    payer: &Keypair,
    chunks_account: &str,
    size: u64
) -> anyhow::Result<Keypair> {

    let chunks_keypair = create_account(rpc_url, payer, chunks_account, size)?;
    let signature = initialize_account(rpc_url, ws_url, payer, &chunks_keypair);
    println!("{:?}",signature);
    Ok(chunks_keypair)
}


#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long, required=true)]
    /// Path to the signer key
    signer: String,

    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value_t=DEFAULT_RPC_URL.to_string())]
    /// URL for solana RPC
    rpc_url: String,

    #[arg(short, long, default_value_t=DEFAULT_WS_URL.to_string())]
    /// URL for solana Websocket
    ws_url: String,
}


#[derive(Subcommand)]
enum Commands {
    #[command(subcommand)]
    /// Manage the chunks account on chain.
    /// This is the scratch space for accumulating chunks on chain scoped to a sequencer
    ChunkAccount(ChunkAccountArgs),
    /// Produce test data (Random bytes of desired size)
    CreateTestData{path: String, size: u64},
    /// Submit chunks to the chain
    Submit{chunk_account: String, blob_file: String},
}

#[derive(Subcommand)]
enum ChunkAccountArgs {
    Create { path: String, size: u64},
    Clear {path: String},
    Info {path: String}
}


fn main() {
    let cli = Cli::parse();
    let signer = cli.signer;
    // let chunks_account = cli.chunks_account;
    // let rpc_url  = cli.rpc_url;
    // let ws_url  = cli.ws_url;
    // let kp = read_keypair_file(signer).unwrap();
    // if !Path::new(&chunks_account).exists() {
    //     create_large_account(&rpc_url, &ws_url,&kp, &chunks_account, 10000000).unwrap();
    // }
    // let ckp = read_keypair_file(chunks_account).unwrap();
    //
    // let raw_data = [1u8;1024];
    // let clist = get_chunks(&raw_data, 100);
    // for c in &clist{
    //     let s = send_chunk_transaction(&rpc_url, &ws_url,&kp, &ckp, c.clone());
    //     println!("{:?}",s);
    // }

}
