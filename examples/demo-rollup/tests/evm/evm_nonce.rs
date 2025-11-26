use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::net::SocketAddr;

use ethereum_types::Address;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::middleware::SignerMiddleware;
use ethers::providers::Http;
use ethers::providers::Middleware;
use ethers::providers::Provider;
use ethers::signers::LocalWallet;
use ethers::signers::Wallet;
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::BlockNumber;
use tokio::try_join;

use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;

type EthersClient = SignerMiddleware<Provider<Http>, Wallet<SigningKey>>;

#[derive(Clone, Debug)]
struct State {
    latest: u64,
    pending: u64,
    default: u64,
    raw_latest: u64,
    raw_pending: u64,
    raw_default: u64,
}

impl Display for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "State {{ latest: {}, pending: {}, default: {}, raw_latest: {}, raw_pending: {}, raw_default: {} }}",
            self.latest, self.pending, self.default, self.raw_latest, self.raw_pending, self.raw_default
        )
    }
}

async fn get_nonces(client: &EthersClient, address: Address) -> anyhow::Result<State> {
    let (latest, pending, default) = try_join!(
        client.get_transaction_count(address, Some(BlockNumber::Latest.into())),
        client.get_transaction_count(address, Some(BlockNumber::Pending.into())),
        client.get_transaction_count(address, None),
    )?;
    let (raw_latest, raw_pending, raw_default) = try_join!(
        client
            .inner()
            .request::<_, String>("eth_getTransactionCount", (address, "latest")),
        client
            .inner()
            .request::<_, String>("eth_getTransactionCount", (address, "pending")),
        client
            .inner()
            .request::<_, String>("eth_getTransactionCount", (address,))
    )?;

    Ok(State {
        latest: latest.as_u64(),
        pending: pending.as_u64(),
        default: default.as_u64(),
        raw_latest: u64::from_str_radix(raw_latest.trim_start_matches("0x"), 16)?,
        raw_pending: u64::from_str_radix(raw_pending.trim_start_matches("0x"), 16)?,
        raw_default: u64::from_str_radix(raw_default.trim_start_matches("0x"), 16)?,
    })
}

async fn ethers_client(socket: SocketAddr) -> anyhow::Result<EthersClient> {
    let http_conn_str = &format!("http://{socket}/rpc");
    let wallet = SENDER_PRIV_KEY.parse::<LocalWallet>().unwrap();
    let provider = Provider::try_from(http_conn_str).unwrap();
    let client = SignerMiddleware::new_with_provider_chain(provider, wallet).await?;
    Ok(client)
}

#[tokio::test(flavor = "multi_thread")]
async fn are_incremented_correctly() -> anyhow::Result<()> {
    // let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    // rollup.wait_for_next_blocks(1).await;
    let client = ethers_client("127.0.0.1:12346".parse()?).await?;
    // let client = ethers_client(rollup.http_addr).await?;

    let nonces = get_nonces(&client, client.address()).await?;
    println!("before all: {nonces}");

    for i in 0..10 {
        println!("\n\n>>>>>>>>");
        let nonces = get_nonces(&client, client.address()).await?;
        println!("before {i}: {nonces}");
        let tx = TypedTransaction::default();
        let pending = client.send_transaction(tx, None).await?;
        let nonces = get_nonces(&client, client.address()).await?;
        println!("sent {i}: {nonces}");
        let _receipt = pending.await?;
        let nonces = get_nonces(&client, client.address()).await?;
        println!("confirmed {i}: {nonces}");
        println!("<<<<<<");
    }

    Ok(())
}
