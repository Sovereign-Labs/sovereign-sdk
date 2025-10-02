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
    },
    /// Run SimpleStorage soak test
    SimpleStorage,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.test {
        TestType::Uniswap { count } => {
            let client = RpcClient::new(&args.private_key, args.rpc_addr).await;
            let signer = Address::from_slice(client.address().as_bytes());
            let test = UniSoakTest::new(client.alloy_client, signer).await?;
            test.run(count).await?;
        }
        TestType::SimpleStorage => {
            let contract = SimpleStorage::default();
            let client = SimpleStorageClient::new(&args.private_key, contract, args.rpc_addr).await;
            simple_storage::run(client).await?;
        }
    }

    Ok(())
}
