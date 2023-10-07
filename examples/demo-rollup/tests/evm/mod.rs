use std::net::SocketAddr;
use std::str::FromStr;

use super::test_helpers::start_rollup;
use demo_stf::genesis_config::GenesisPaths;
use ethers_core::abi::Address;
use ethers_signers::{LocalWallet, Signer};
use sov_evm::SimpleStorageContract;
use sov_risc0_adapter::host::Risc0Host;

mod test_client;
use test_client::TestClient;

const TEST_GENESIS_PATHS: GenesisPaths<&str> = GenesisPaths {
    bank_genesis_path: "../test-data/genesis/integration-tests/bank.json",
    sequencer_genesis_path: "../test-data/genesis/integration-tests/sequencer_registry.json",
    value_setter_genesis_path: "../test-data/genesis/integration-tests/value_setter.json",
    accounts_genesis_path: "../test-data/genesis/integration-tests/accounts.json",
    chain_state_genesis_path: "../test-data/genesis/integration-tests/chain_state.json",
    nft_path: "../test-data/genesis/integration-tests/nft.json",
    #[cfg(feature = "experimental")]
    evm_genesis_path: "../test-data/genesis/integration-tests/evm.json",
};

async fn send_tx_test_to_eth(rpc_address: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let chain_id: u64 = 1;
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let contract = SimpleStorageContract::default();

    let from_addr = Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();

    let test_client = TestClient::new(chain_id, key, from_addr, contract, rpc_address).await;

    let etc_accounts = test_client.eth_accounts().await;
    assert_eq!(vec![from_addr], etc_accounts);

    let eth_chain_id = test_client.eth_chain_id().await;
    assert_eq!(chain_id, eth_chain_id);

    // No block exists yet
    let latest_block = test_client
        .eth_get_block_by_number(Some("latest".to_owned()))
        .await;
    let earliest_block = test_client
        .eth_get_block_by_number(Some("earliest".to_owned()))
        .await;

    assert_eq!(latest_block, earliest_block);
    assert_eq!(latest_block.number.unwrap().as_u64(), 0);

    test_client.execute().await
}

#[cfg(feature = "experimental")]
#[tokio::test]
async fn evm_tx_tests() -> Result<(), anyhow::Error> {
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    let rollup_task = tokio::spawn(async {
        // Don't provide a prover since the EVM is not currently provable
        start_rollup::<Risc0Host<'static>, _>(port_tx, None, &TEST_GENESIS_PATHS).await;
    });

    // Wait for rollup task to start:
    let port = port_rx.await.unwrap();
    send_tx_test_to_eth(port).await.unwrap();
    rollup_task.abort();
    Ok(())
}
