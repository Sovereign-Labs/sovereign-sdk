use alloy::hex;
use alloy_primitives::Address;
use anyhow::Result;
use clap::Parser;
use sov_eth_client::{RpcClient, SimpleStorageClient};
use sov_test_utils::SimpleStorage;
use std::net::SocketAddr;

use crate::uniswap::UniSoakTest;

mod simple_storage;
mod uniswap;

#[derive(Parser, Debug)]
#[command(name = "sov-evm-soak-testing")]
#[command(about = "EVM soak testing tool", long_about = None)]
struct Args {
    /// RPC address
    #[arg(short, long, default_value = "127.0.0.1:12346")]
    rpc_addr: SocketAddr,

    /// Private key for signing transactions
    #[arg(
        short,
        long,
        default_value = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    )]
    private_key: String,

    #[command(subcommand)]
    test: TestType,

    #[arg(short, long, default_value = "1")]
    num_workers: usize,
}

#[derive(clap::Subcommand, Clone, Debug)]
enum TestType {
    /// Run Uniswap soak test
    Uniswap {
        /// Number of iterations
        #[arg(short, long, default_value = "100")]
        count: usize,
    },
    /// Run SimpleStorage soak test
    SimpleStorage,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>> =
        Vec::with_capacity(args.num_workers);
    match args.test {
        TestType::Uniswap { count } => {
            for i in 0..args.num_workers {
                // Tweak the private key to avoid conflicts between workers
                let mut key_bytes: [u8; 32] =
                    hex::decode(&args.private_key).unwrap().try_into().unwrap();
                key_bytes[0] = key_bytes[0].wrapping_add(i as u8);
                let key = hex::encode(key_bytes);
                // Clone the RPC address to avoid lifetime issues
                let addr = args.rpc_addr.clone();
                // Spawn a new task for each worker
                handles.push(tokio::spawn(async move {
                    let client = RpcClient::new(&key, addr).await;
                    let signer = Address::from_slice(&client.address().0);
                    match UniSoakTest::new(client.alloy_client, signer).await {
                        Ok(test) => {
                            if let Err(e) = test.run(count).await {
                                println!("Worker {i} error during run: {e:?}");
                            }
                        }
                        Err(e) => {
                            println!("Worker {i} failed to deploy contracts: {e:?}");
                        }
                    }
                    Ok(())
                }));
            }
        }
        TestType::SimpleStorage => {
            let contract = SimpleStorage::default();
            let client = SimpleStorageClient::new(&args.private_key, contract, args.rpc_addr).await;
            simple_storage::run(client).await?;
        }
    }
    for handle in handles {
        handle.await??;
    }

    Ok(())
}
