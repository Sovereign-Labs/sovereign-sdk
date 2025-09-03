use super::evm_test_helper;
use crate::test_helpers::DemoRollupSpec;
use ethers_core::abi::Address;
use sov_address::{EthereumAddress, MultiAddress};
use sov_bank::config_gas_token_id;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup, MockRollupSpec};
use sov_modules_api::execution_mode::Native;
use sov_modules_macros::config_value;
use sov_test_utils::test_rollup::{self, get_appropriate_rollup_prover_config};
use std::str::FromStr;

const SENDER_PRIV_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SENDER_ADDR_STR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

const RECIEVER_ADDR_STR: &str = "0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71";

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_soft_confirmations() -> anyhow::Result<()> {
    let rollup_prover_config =
        get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(mock_da_risc0_host_args());

    let chain_id = config_value!("CHAIN_ID");
    let test_rollup = evm_test_helper::start_node(rollup_prover_config, 0).await;

    let (evm_client, _) =
        evm_test_helper::create_test_client(test_rollup.http_addr, chain_id, SENDER_PRIV_KEY).await;

    test_rollup.wait_for_next_blocks(10).await;

    let sender_address = Address::from_str(SENDER_ADDR_STR).unwrap();
    let reciever_address = Address::from_str(RECIEVER_ADDR_STR).unwrap();

    let eth_to_send = 2;
    let pending_tx = evm_client.send_eth(reciever_address, eth_to_send).await;

    let tx_hash = pending_tx.tx_hash();
    //pending_tx.await;
    test_rollup.wait_for_next_blocks(2).await;

    let rec = evm_client.receipt(tx_hash).await;

    println!("{:?}", rec);

    test_rollup.wait_for_next_blocks(2).await;

    Ok(())
}
