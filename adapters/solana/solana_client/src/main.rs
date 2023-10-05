use {
    anchor_client::{
        Client,
        Cluster,
    },
    solana_sdk::{
        signer::keypair::read_keypair_file,
        pubkey::Pubkey,
    },
    anchor_lang::prelude::*,
};

use blockroot::accounts as blockroot_accounts;
use blockroot::instruction as blockroot_instruction;

use clap::Parser;
use alloc::rc::Rc;
extern crate alloc;

const DEFAULT_URL: &str = "http://127.0.0.1:8899";

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
        &bs58::decode("669V5MzeTYri4kmbuY794EMxcKY2LKHZQzL9recSdPUL").into_vec().unwrap());
    let prog = c.program(program_id).unwrap();
    let signature = prog.request()
        .accounts(blockroot_accounts::Initialize {})
        .args(blockroot_instruction::Initialize {})
        .send();
    println!("{:?}",signature);
}
