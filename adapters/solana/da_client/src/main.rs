use alloc::rc::Rc;
use std::path::Path;
use std::process;
use std::str::FromStr;

use anchor_client::{Client, Cluster};
use anchor_lang::solana_program::sysvar::clock::Clock;
use blockroot::da::{get_chunks, Chunk, ChunkAccumulator, CHUNK_SIZE, PREFIX};
use blockroot::{accounts as blockroot_accounts, instruction as blockroot_instruction};
use clap::{Parser, Subcommand};
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{EncodableKey, Keypair, Signature, Signer};
use solana_sdk::signer::keypair::read_keypair_file;
use solana_sdk::sysvar::SysvarId;
use solana_sdk::transaction::Transaction;
use solana_sdk::{system_instruction, system_program};
extern crate alloc;

use da_client::{read_file_to_vec, write_random_bytes};

const DEFAULT_RPC_URL: &str = "http://localhost:8899";
const DEFAULT_WS_URL: &str = "ws://localhost:8900";

pub struct DaClient {
    pub rpc_url: String,
    pub ws_url: String,
    pub signer: Keypair,
    pub blockroot_program: Pubkey,
    pub blockroot_pda: (Pubkey, u8),
    pub clock_account: Pubkey,
    pub system_program: Pubkey,
}

impl DaClient {
    pub fn new(rpc_url: String, ws_url: String, signer: Keypair, blockroot_program: &str) -> Self {
        let blockroot_program_pubkey = Pubkey::from_str(blockroot_program).unwrap();
        let (blockroot_pda, bump) =
            Pubkey::find_program_address(&[PREFIX.as_bytes()], &blockroot_program_pubkey);

        DaClient {
            rpc_url,
            ws_url,
            signer,
            blockroot_program: Pubkey::from_str(blockroot_program).unwrap(),
            blockroot_pda: (blockroot_pda, bump),
            clock_account: Clock::id(),
            system_program: system_program::id(),
        }
    }

    pub fn create_account(&self, chunks_account: &str, size: u64) -> anyhow::Result<Keypair> {
        let client = RpcClient::new(&self.rpc_url);
        let new_account = Keypair::new();
        new_account.write_to_file(chunks_account).unwrap();

        let rent = client.get_minimum_balance_for_rent_exemption(size.try_into()?)?;
        let instr = system_instruction::create_account(
            &self.signer.pubkey(),
            &new_account.pubkey(),
            rent,
            size,
            &self.blockroot_program,
        );

        let blockhash = client.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &[instr],
            Some(&self.signer.pubkey()),
            &[&self.signer, &new_account],
            blockhash,
        );
        client.send_and_confirm_transaction(&tx)?;
        Ok(new_account)
    }

    pub fn initialize_account(&self, chunks_keypair: &Keypair) -> anyhow::Result<Signature> {
        let c = Client::new(
            Cluster::Custom(self.rpc_url.clone(), self.ws_url.clone()),
            Rc::new(self.signer.insecure_clone()),
        );
        let prog = c.program(self.blockroot_program).unwrap();

        let signature = prog
            .request()
            .accounts(blockroot_accounts::Initialize {
                creator: self.signer.pubkey(),
                chunk_accumulator: chunks_keypair.pubkey(),
                system_program: self.system_program,
            })
            .args(blockroot_instruction::Initialize {})
            .signer(chunks_keypair)
            .send()?;
        Ok(signature)
    }

    pub fn create_large_account(&self, chunks_account: &str, size: u64) -> anyhow::Result<Keypair> {
        let chunks_keypair = self.create_account(chunks_account, size)?;
        let signature = self.initialize_account(&chunks_keypair);
        println!("{:?}", signature);
        Ok(chunks_keypair)
    }

    pub fn send_chunk_transaction(
        &self,
        chunks_keypair: &Keypair,
        chunk: Chunk,
    ) -> anyhow::Result<Signature> {
        let creator_pubkey = self.signer.pubkey();
        let c = Client::new(
            Cluster::Custom(self.rpc_url.clone(), self.ws_url.clone()),
            Rc::new(self.signer.insecure_clone()),
        );
        let prog = c.program(self.blockroot_program).unwrap();

        let signature = prog
            .request()
            .accounts(blockroot_accounts::ProcessChunk {
                creator: creator_pubkey,
                chunk_accumulator: chunks_keypair.pubkey(),
                clock: self.clock_account,
                blocks_root: self.blockroot_pda.0,
                system_program: self.system_program,
            })
            .args(blockroot_instruction::ProcessChunk {
                bump: self.blockroot_pda.1,
                chunk,
            })
            .options(CommitmentConfig {
                commitment: CommitmentLevel::Processed,
            })
            .signer(chunks_keypair)
            .send()?;
        Ok(signature)
    }

    pub fn wipe_account(&self, chunks_keypair: &Keypair) -> anyhow::Result<Signature> {
        let creator_pubkey = self.signer.pubkey();
        let c = Client::new(
            Cluster::Custom(self.rpc_url.clone(), self.ws_url.clone()),
            Rc::new(self.signer.insecure_clone()),
        );
        let prog = c.program(self.blockroot_program).unwrap();

        let signature = prog
            .request()
            .accounts(blockroot_accounts::Clear {
                creator: creator_pubkey,
                chunk_accumulator: chunks_keypair.pubkey(),
                system_program: self.system_program,
            })
            .args(blockroot_instruction::Clear { digest: None })
            .signer(chunks_keypair)
            .send()?;
        Ok(signature)
    }
}

fn _accumulate_chunks_get_root(chunks: Vec<Chunk>) -> Option<[u8; 32]> {
    let raw_data_digest = chunks[0].digest;
    let mut ca = ChunkAccumulator::new();
    for c in chunks {
        ca.accumulate(c);
    }
    ca.get_merkle_root(&raw_data_digest)
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long, required = true)]
    /// Path to the signer key
    signer: String,

    #[arg(long, required = true)]
    /// b58 encoded address for the on chain sovereign blockroot program
    blockroot_program: String,

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
    CreateTestData {
        test_blob_path: String,
        size: u64,
    },
    /// Submit chunks to the chain
    Submit {
        chunk_account_path: String,
        blob_path: String,
    },
    Verify,
}

#[derive(Subcommand)]
enum ChunkAccountArgs {
    Create {
        path: String,
        #[arg(short, long, default_value_t = 10000000)]
        size: u64,
        #[arg(short, long, default_value_t = false)]
        force: bool,
        #[arg(short, long, default_value_t = false)]
        use_existing: bool,
    },
    Clear {
        path: String,
    },
    Info {
        path: String,
    },
}

fn main() {
    let cli = Cli::parse();

    // required parameters
    let signer = cli.signer;
    let blockroot_program = &cli.blockroot_program;

    // optional overrides
    let rpc_url = cli.rpc_url;
    let ws_url = cli.ws_url;

    let signer_keypair = read_keypair_file(signer).unwrap();

    let da_client = DaClient::new(rpc_url, ws_url, signer_keypair, blockroot_program);

    // Cli parsing
    match &cli.command {
        Commands::Verify => {
            // TODO: we want to be able get the block data and the root from validator
            // and verify that the data provided matches what was published on-chain
            unimplemented!()
        }
        Commands::ChunkAccount(chunk_args) => match chunk_args {
            ChunkAccountArgs::Create {
                path,
                size,
                force,
                use_existing,
            } => {
                if Path::new(path).exists() {
                    if *force {
                        println!("Over-writing existing keypair at {} ", path);
                        da_client.create_large_account(&path, *size).unwrap();
                    } else {
                        if *use_existing {
                            println!("Attempting to re-use existing keypair at {} ", path);
                            let chunks_keypair = read_keypair_file(path).unwrap();
                            let signature = da_client.initialize_account(&chunks_keypair).unwrap();
                            println!("{}", signature);
                        }
                        println!("Chunk account keypair already exists. Use \
                                 --force to create a new keypair and override existing one, or \
                                 --use_existing to use the existing file, fund it and transfer ownership to blockroot program");
                        process::exit(1);
                    }
                } else {
                    da_client.create_large_account(&path, *size).unwrap();
                }
            }
            ChunkAccountArgs::Clear { path } => {
                let chunks_keypair = read_keypair_file(path).unwrap();
                da_client.wipe_account(&chunks_keypair).unwrap();
            }
            ChunkAccountArgs::Info { path: _path } => {
                unimplemented!()
            }
        },
        Commands::CreateTestData {
            test_blob_path,
            size,
        } => {
            write_random_bytes(test_blob_path, *size).unwrap();
        }
        Commands::Submit {
            chunk_account_path,
            blob_path,
        } => {
            let chunks_keypair = read_keypair_file(chunk_account_path).unwrap();
            let contents = read_file_to_vec(blob_path).expect("Failed to read from the file");
            let chunk_list = get_chunks(&contents, CHUNK_SIZE);
            println!("raw data file: {}", blob_path);
            println!("digest: {}", hex::encode(chunk_list[0].digest));
            println!("number of chunk transactions: {}", chunk_list[0].num_chunks);
            println!(
                "chunks digest for blob file at {} is {} ",
                blob_path,
                hex::encode(chunk_list[0].digest)
            );
            for c in chunk_list {
                let sig = da_client.send_chunk_transaction(&chunks_keypair, c);
                println!("{:?}", sig);
            }
        }
    }
}
