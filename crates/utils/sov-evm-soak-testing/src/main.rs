use crate::{logs::run_logs_test, uniswap::UniSoakTest};
use alloy::network::TransactionBuilder;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use alloy::{hex, providers::DynProvider};
use alloy_primitives::U256;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use futures::future::try_join_all;
use reqwest::Url;
use std::net::SocketAddr;

mod logs;
mod simple_storage;
mod uniswap;

/// Maximum number of concurrent workers supported due to private key derivation constraints.
const MAX_WORKERS: usize = 255;

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

        /// Number of parallel workers to spawn
        #[arg(short, long, default_value = "1")]
        num_workers: usize,
    },
    /// Run SimpleStorage soak test
    SimpleStorage,
    /// Run logs soak test
    Logs {
        /// Number of transactions to send
        #[arg(short, long, default_value = "100")]
        tx_count: usize,

        /// Number of logs to emit per transaction
        #[arg(short, long, default_value = "100")]
        logs_per_tx: usize,

        /// Number of parallel workers to spawn
        #[arg(short, long, default_value = "1")]
        num_workers: usize,

        #[command(subcommand)]
        mode: LogsRetrievalMode,
    },
}

#[derive(Subcommand, Clone, Debug)]
enum LogsRetrievalMode {
    /// Retrieve logs using eth_subscribe
    Subscription {
        /// Channel capacity for the subscription
        #[arg(short, long, default_value = "100000")]
        capacity: usize,
    },
    /// Retrieve logs using cursor-based pagination
    WithCursor,
}

/// Derives a unique private key for a worker by tweaking the root key.
///
/// This modifies the first byte of the root key to ensure each worker has a distinct
/// account and avoids nonce conflicts when running parallel tests.
fn derive_worker_key(root_key: &str, worker_idx: usize) -> Result<String> {
    let mut key_bytes: [u8; 32] = hex::decode(root_key)?
        .try_into()
        .map_err(|_| anyhow!("Invalid private key length"))?;

    key_bytes[0] = key_bytes[0].wrapping_add(worker_idx as u8);
    Ok(hex::encode(key_bytes))
}

/// Creates an Alloy HTTP client connected to the specified RPC server.
pub(crate) fn alloy_client(rpc_addr: SocketAddr, signer: PrivateKeySigner) -> Result<DynProvider> {
    let url = Url::parse(&format!("http://{rpc_addr}/rpc"))?;
    let client = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased();
    Ok(client)
}

/// Creates an Alloy WS client connected to the specified RPC server.
pub(crate) async fn alloy_ws_client(
    rpc_addr: SocketAddr,
    signer: PrivateKeySigner,
) -> Result<DynProvider> {
    let url = Url::parse(&format!("ws://{rpc_addr}/rpc"))?;
    let ws = WsConnect::new(url);
    let client = ProviderBuilder::new()
        .wallet(signer)
        .connect_ws(ws)
        .await
        .unwrap()
        .erased();
    Ok(client)
}

/// Validates that the number of workers doesn't exceed the maximum supported.
fn validate_worker_count(num_workers: usize) -> Result<()> {
    if num_workers > MAX_WORKERS {
        return Err(anyhow!(
            "num_workers must be at most {MAX_WORKERS} due to private key derivation constraints"
        ));
    }
    Ok(())
}

/// Spawns multiple Uniswap test workers and waits for them to complete.
async fn run_uniswap_test(
    rpc_addr: SocketAddr,
    private_key: &str,
    count: usize,
    num_workers: usize,
) -> Result<()> {
    validate_worker_count(num_workers)?;

    let mut handles = Vec::with_capacity(num_workers);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let client = alloy_client(rpc_addr, signer.clone())?;

        handles.push(tokio::spawn(async move {
            match UniSoakTest::new(client, signer.address()).await {
                Ok(test) => {
                    if let Err(e) = test.run(count).await {
                        eprintln!("Worker {worker_idx} error during run: {e:?}");
                    }
                }
                Err(e) => {
                    eprintln!("Worker {worker_idx} failed to deploy contracts: {e:?}");
                }
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    try_join_all(handles).await?;
    Ok(())
}

/// Funds worker accounts from the root account.
async fn fund_worker_accounts(
    root_client: &DynProvider,
    root_signer: &PrivateKeySigner,
    private_key: &str,
    num_workers: usize,
) -> Result<()> {
    let root_balance = root_client.get_balance(root_signer.address()).await?;
    let transfer_amount = root_balance.wrapping_div(U256::from(num_workers));

    for worker_idx in 0..num_workers {
        let worker_signer: PrivateKeySigner =
            derive_worker_key(private_key, worker_idx)?.parse()?;
        let tx = TransactionRequest::default()
            .with_from(root_signer.address())
            .with_to(worker_signer.address())
            .with_value(transfer_amount);

        root_client.send_transaction(tx).await?.watch().await?;
    }
    Ok(())
}

/// Runs the SimpleStorage soak test.
async fn run_simple_storage_test(rpc_addr: SocketAddr, private_key: &str) -> Result<()> {
    let signer: PrivateKeySigner = private_key.parse()?;
    let client = alloy_client(rpc_addr, signer)?;
    simple_storage::run(client).await
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.test {
        TestType::Uniswap { count, num_workers } => {
            run_uniswap_test(args.rpc_addr, &args.private_key, count, num_workers).await?;
        }
        TestType::SimpleStorage => {
            run_simple_storage_test(args.rpc_addr, &args.private_key).await?;
        }
        TestType::Logs {
            tx_count,
            logs_per_tx,
            num_workers,
            mode,
        } => {
            run_logs_test(
                args.rpc_addr,
                &args.private_key,
                tx_count,
                logs_per_tx,
                num_workers,
                mode,
            )
            .await?;
        }
    }

    Ok(())
}
