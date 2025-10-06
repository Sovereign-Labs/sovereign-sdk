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
}

#[derive(clap::Subcommand, Clone, Debug)]
enum TestType {
    /// Run Uniswap soak test
    Uniswap {
        /// Number of iterations
        #[arg(short, long, default_value = "100")]
        count: usize,

        #[arg(short, long, default_value = "1")]
        num_workers: usize,
    },
    /// Run SimpleStorage soak test
    SimpleStorage,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.test {
        TestType::Uniswap { count, num_workers } => {
            if num_workers > 255 {
                return Err(anyhow::anyhow!("num_workers must be less than 256 because of our private key tweaking. This is an easy fix, but we haven't done it yet."));
            }
            let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>> =
                Vec::with_capacity(num_workers);
            for i in 0..num_workers {
                // Tweak the private key to avoid conflicts between workers
                let key = {
                    let mut key_bytes: [u8; 32] =
                        hex::decode(&args.private_key).unwrap().try_into().unwrap();
                    key_bytes[0] = key_bytes[0].wrapping_add(i as u8);
                    hex::encode(key_bytes)
                };
                // Spawn a new task for each worker
                handles.push(tokio::spawn(async move {
                    let client = RpcClient::new(&key, args.rpc_addr).await;
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

            for handle in handles {
                handle.await??;
            }
        }
        TestType::SimpleStorage => {
            let contract = SimpleStorage::default();
            let client = SimpleStorageClient::new(&args.private_key, contract, args.rpc_addr).await;
            simple_storage::run(client).await?;
        }
    }

    Ok(())
}
