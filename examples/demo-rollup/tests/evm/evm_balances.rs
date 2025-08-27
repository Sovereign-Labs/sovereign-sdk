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
async fn evm_test_balances() -> anyhow::Result<()> {
    let rollup_prover_config =
        get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(mock_da_risc0_host_args());

    let chain_id = config_value!("CHAIN_ID");
    let test_rollup = evm_test_helper::start_node(rollup_prover_config, 0).await;

    let (evm_client, _) =
        evm_test_helper::create_test_client(test_rollup.http_addr, chain_id, SENDER_PRIV_KEY).await;

    test_rollup.wait_for_next_blocks(10).await;

    let sender_address = Address::from_str(SENDER_ADDR_STR).unwrap();
    let reciever_address = Address::from_str(RECIEVER_ADDR_STR).unwrap();

    let (snder_bank_balance_start, sender_evm_balance_start) =
        get_balances(sender_address, &test_rollup, &evm_client).await;

    let (reciever_bank_balance_start, reciever_evm_balance_start) =
        get_balances(reciever_address, &test_rollup, &evm_client).await;

    let eth_to_send = 2;
    evm_client.send_eth(reciever_address, eth_to_send).await;
    test_rollup.wait_for_next_blocks(2).await;

    let (snder_bank_balance_end, sender_evm_balance_end) =
        get_balances(sender_address, &test_rollup, &evm_client).await;

    let (reciever_bank_balance_end, reciever_evm_balance_end) =
        get_balances(reciever_address, &test_rollup, &evm_client).await;

    // ASSERTIONS:

    // Sender
    assert_eq!(snder_bank_balance_start, sender_evm_balance_start);
    assert_eq!(snder_bank_balance_end, sender_evm_balance_end);

    //  Sender also pays gas, so the balance check uses `>`
    assert!(snder_bank_balance_start > snder_bank_balance_end + eth_to_send);

    // Reciever
    assert_eq!(reciever_bank_balance_start, reciever_evm_balance_start);
    assert_eq!(
        reciever_bank_balance_start + eth_to_send,
        reciever_bank_balance_end
    );
    assert_eq!(
        reciever_evm_balance_start + eth_to_send,
        reciever_evm_balance_end
    );

    Ok(())
}

async fn get_balances(
    address: Address,
    test_rollup: &test_rollup::TestRollup<MockDemoRollup<Native>>,
    evm_client: &sov_eth_client::TestClient,
) -> (u128, u128) {
    let sov_to_addr = MultiAddress::Vm(EthereumAddress::new(address.0));

    let token_id = config_gas_token_id();

    let bank_balance = test_rollup
        .client
        .get_balance::<DemoRollupSpec>(&sov_to_addr, &token_id, None)
        .await
        .unwrap()
        .0;

    let evm_balance = evm_client.eth_get_balance(address).await.as_u128();
    (bank_balance, evm_balance)
}
