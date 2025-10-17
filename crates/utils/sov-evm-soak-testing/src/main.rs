use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::{hex, providers::DynProvider};
use anyhow::{anyhow, Result};
use clap::Parser;
use clap::Subcommand;
use futures::future::try_join_all;
use reqwest::Url;
use std::net::SocketAddr;
use tokio::task::JoinHandle;

use crate::{logs::LogsSoakTest, uniswap::UniSoakTest};

mod logs;
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

#[derive(Subcommand, Clone, Debug)]
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
    /// Run Logs soak test
    Logs {
        /// Number of txs
        #[arg(short, long, default_value = "100")]
        tx_count: usize,
        /// Number of logs per tx
        #[arg(short, long, default_value = "100")]
        logs_per_tx: usize,
    },
}

// Tweak the private key to avoid conflicts between workers
fn derive_worker_key(root_key: &str, idx: usize) -> Result<String> {
    let mut key_bytes: [u8; 32] = hex::decode(root_key)?.try_into().unwrap();
    key_bytes[0] = key_bytes[0].wrapping_add(idx as u8);
    Ok(hex::encode(key_bytes))
}

pub(crate) fn alloy_client(socket: SocketAddr, signer: PrivateKeySigner) -> Result<DynProvider> {
    let url = Url::parse(&format!("http://{socket}/rpc"))?;
    let client = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased();
    Ok(client)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.test {
        TestType::Uniswap { count, num_workers } => {
            if num_workers > 255 {
                return Err(anyhow!("num_workers must be less than 256 because of our private key tweaking. This is an easy fix, but we haven't done it yet."));
            }
            let mut handles: Vec<JoinHandle<Result<()>>> = Vec::with_capacity(num_workers);
            for i in 0..num_workers {
                let signer: PrivateKeySigner = derive_worker_key(&args.private_key, i)?.parse()?;
                let client = alloy_client(args.rpc_addr, signer.clone())?;
                // Spawn a new task for each worker
                handles.push(tokio::spawn(async move {
                    match UniSoakTest::new(client, signer.address()).await {
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
            try_join_all(handles).await?;
        }
        TestType::SimpleStorage => {
            let signer: PrivateKeySigner = args.private_key.parse()?;
            let client = alloy_client(args.rpc_addr, signer)?;
            simple_storage::run(client).await?;
        }
        TestType::Logs {
            tx_count,
            logs_per_tx,
        } => {
            let signer: PrivateKeySigner = args.private_key.parse()?;
            let client = alloy_client(args.rpc_addr, signer)?;
            let test = LogsSoakTest::new(client).await?;
            test.run(tx_count, logs_per_tx).await?;
        }
    }

    Ok(())
}
