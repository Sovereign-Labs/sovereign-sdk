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
use tracing::warn;
use tracing_subscriber::EnvFilter;

mod logs;
pub(crate) mod recv_many;
mod simple_storage;
mod state_writer;
mod uniswap;
mod transfer;

/// Maximum number of concurrent workers supported due to private key derivation constraints.
const MAX_WORKERS: usize = 255;

#[derive(Parser, Debug)]
#[command(name = "sov-evm-soak-testing")]
#[command(about = "EVM soak testing tool", long_about = None)]
struct Args {
    /// RPC address
    #[arg(short, long, default_value = "127.0.0.1:12348")]
    rpc_addr: String,

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
    StateWriter {
        /// Number of transactions to send
        #[arg(short, long, default_value = "1000")]
        tx_count: usize,

        /// Number of logs to emit per transaction
        #[arg(short, long, default_value = "100")]
        writes_per_tx: usize,

        /// Number of parallel workers to spawn
        #[arg(short, long, default_value = "1")]
        num_workers: usize,

        /// How many of the writes should overlap from one tx to the next
        #[arg(short, long, default_value = "0")]
        overlapping_writes_per_tx: usize,
    },
    Transfer {
        /// Number of transactions to send
        #[arg(short, long, default_value = "1000")]
        tx_count: usize,

        /// Number of parallel workers to spawn
        #[arg(short, long, default_value = "1")]
        num_workers: usize,
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

    let offset = (worker_idx as u16).to_le_bytes();
    key_bytes[0] = key_bytes[0].wrapping_add(offset[0]);
    key_bytes[1] = key_bytes[1].wrapping_add(offset[1]);
    Ok(hex::encode(key_bytes))
}

/// Creates an Alloy HTTP client connected to the specified RPC server.
pub(crate) fn alloy_client(rpc_addr: String, signer: PrivateKeySigner) -> Result<DynProvider> {
    let url = Url::parse(&format!("{rpc_addr}/rpc"))?;
    let client = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased();
    Ok(client)
}

/// Creates an Alloy WS client connected to the specified RPC server.
pub(crate) async fn alloy_ws_client(
    rpc_addr: String,
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
    rpc_addr: String,
    private_key: &str,
    count: usize,
    num_workers: usize,
) -> Result<()> {
    validate_worker_count(num_workers)?;

    let mut handles = Vec::with_capacity(num_workers);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let client = alloy_client(rpc_addr.clone(), signer.clone())?;

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
    if root_balance == U256::ZERO {
        warn!("Root balance is 0. Skipping funding. This is fine if the paymaster is enabled.");
        return Ok(());
    }
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
async fn run_simple_storage_test(rpc_addr: String, private_key: &str) -> Result<()> {
    let signer: PrivateKeySigner = private_key.parse()?;
    let client = alloy_client(rpc_addr, signer)?;
    simple_storage::run(client).await
}

/// Runs the StateWriter soak test.
async fn run_state_writer_test(
    rpc_addr: String,
    private_key: &str,
    tx_count: usize,
    writes_per_tx: usize,
    num_workers: usize,
    overlapping_writes_per_tx: usize,
) -> Result<()> {
    let mut handles = Vec::with_capacity(num_workers);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let address = signer.address();
        let client = alloy_client(rpc_addr.clone(), signer.clone())?;
        let tx_sender = tx.clone();

        handles.push(tokio::spawn(async move {
            let res = state_writer::run(
                client,
                tx_count,
                writes_per_tx,
                overlapping_writes_per_tx,
                address,
            )
            .await;
            if let Err(e) = &res {
                println!("Worker {worker_idx} error during run: {e:?}");
            }
            drop(tx_sender);
            res
        }));
    }
    drop(tx);

    let handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    let completed_txs = state_writer::COMPLETED_TXS.load(std::sync::atomic::Ordering::Relaxed);
                    println!("Completed {completed_txs} ({} writes) TXs in {}ms. {} Writes/s", writes_per_tx * completed_txs, start.elapsed().as_millis(), ((completed_txs * writes_per_tx) as f64) / start.elapsed().as_secs_f64());
                }
                _ = rx.recv() => {
                    break;
                }
            }
        }
        Ok(())
    });
    handles.push(handle);
    try_join_all(handles).await?;
    Ok(())
}


/// Runs the StateWriter soak test.
async fn run_transfer_test(
    rpc_addr: String,
    private_key: &str,
    tx_count: usize,
    num_workers: usize,
) -> Result<()> {
    let funding_signer: PrivateKeySigner = "0x0d87c12ea7c12024b3f70a26d735874608f17c8bce2b48e6fe87389310191264".parse()?;
    let funding_address = funding_signer.address();
    let funding_client = alloy_client(rpc_addr.clone(), funding_signer.clone())?;
    let funding_client_nonce = funding_client.get_transaction_count(funding_address).await?;
    transfer::FUNDING_CLIENT_NONCE.store(funding_client_nonce, std::sync::atomic::Ordering::SeqCst);

    let mut handles = Vec::with_capacity(num_workers);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    for worker_idx in 0..num_workers {
        let signer: PrivateKeySigner = derive_worker_key(private_key, worker_idx)?.parse()?;
        let address = signer.address();
        tracing::info!("Worker {worker_idx} address: {address}");
        let client = alloy_client(rpc_addr.clone(), signer.clone())?;
        let tx_sender = tx.clone();
        let funding_client = funding_client.clone();
       

        handles.push(tokio::spawn(async move {
            let res = transfer::run(
                client,
                funding_client,
                tx_count,
                address,
            )
            .await;
            if let Err(e) = &res {
                println!("Worker {worker_idx} error during run: {e:?}");
            }
            drop(tx_sender);
            res
        }));
    }
    drop(tx);

    let handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    let completed_txs = transfer::COMPLETED_TXS.load(std::sync::atomic::Ordering::Relaxed);
                    println!("Completed {completed_txs} TXs in {}ms. {} TXs/s", start.elapsed().as_millis(), (completed_txs as f64) / start.elapsed().as_secs_f64());
                }
                _ = rx.recv() => {
                    break;
                }
            }
        }
        Ok(())
    });
    handles.push(handle);
    try_join_all(handles).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or("debug".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();
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
        TestType::StateWriter {
            tx_count,
            writes_per_tx,
            num_workers,
            overlapping_writes_per_tx,
        } => {
            run_state_writer_test(
                args.rpc_addr,
                &args.private_key,
                tx_count,
                writes_per_tx,
                num_workers,
                overlapping_writes_per_tx,
            )
            .await?;
        }
        TestType::Transfer { tx_count, num_workers } => {
            run_transfer_test(args.rpc_addr, &args.private_key, tx_count, num_workers).await?;
        }
    }

    Ok(())
}
