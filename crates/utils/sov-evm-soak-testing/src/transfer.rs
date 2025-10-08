use crate::helpers::generate_priv_keys;
use crate::RpcClient;
use std::net::SocketAddr;

pub(crate) async fn run(
    count: usize,
    foucet_private_key: &str,
    rpc_addr: SocketAddr,
) -> anyhow::Result<()> {
    let priv_keys: Vec<_> = generate_priv_keys(count, foucet_private_key)?
        .into_iter()
        .map(|pk| (RpcClient::alloy_address(&pk), pk))
        .collect();

    // Send funds to all participants in the benchmark.
    let foucet_client = RpcClient::new(&foucet_private_key, rpc_addr).await;
    let block_number = foucet_client.block_number().await;
    for (addr, _) in priv_keys.iter() {
        foucet_client.alloy_send_eth(*addr, 2).await;
    }

    // Wait for a new block once all participants have been funded.
    // This ensures that all block-lifetime caches are reset before the next phase of the benchmark.
    loop {
        let current_block_number = foucet_client.block_number().await;
        if current_block_number > block_number {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // The participants simultaneously return a portion of the funds to the faucet.
    let foucet_address = RpcClient::alloy_address(foucet_private_key);
    for (_, pk) in priv_keys.into_iter() {
        tokio::spawn(async move {
            let client = RpcClient::new(&pk, rpc_addr).await;
            client.alloy_send_eth(foucet_address, 1).await;
            dbg!(client.address());
        });
    }

    Ok(())
}
