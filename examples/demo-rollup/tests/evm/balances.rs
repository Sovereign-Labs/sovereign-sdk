use std::time::Duration;

use super::evm_test_helper;
use ethers_core::abi::Address;
use full_node_configs::sequencer::default_ideal_lag_behind_finalized_slot;
use sov_address::{EthereumAddress, MultiAddress};
use sov_bank::config_gas_token_id;
use sov_demo_rollup::{mock_da_risc0_host_args, MockRollupSpec};
use sov_eth_client::TestClient;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_modules_macros::config_value;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::initialize_logging;
use sov_test_utils::test_rollup::get_appropriate_rollup_prover_config;
use std::str::FromStr;
use tokio::time::sleep;

use crate::test_helpers::DemoRollupSpec;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_balances() -> anyhow::Result<()> {
    //sov_test_utils::initialize_logging();
    let rollup_prover_config =
        get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(mock_da_risc0_host_args());

    let chain_id = config_value!("CHAIN_ID");
    let test_rollup = evm_test_helper::start_node(rollup_prover_config, 0).await;

    let (client, _) = evm_test_helper::create_test_client(
        test_rollup.http_addr,
        chain_id,
        // corresponding pub key 0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    )
    .await;

    let chain_id = config_value!("CHAIN_ID");
    // temp_dir is hold here os it is not removed during test run

    test_rollup
        .da_service
        .produce_n_blocks_now(10 + default_ideal_lag_behind_finalized_slot() as usize)
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    test_rollup.da_service.produce_n_blocks_now(1).await?;
    sleep(Duration::from_secs(1)).await;

    dbg!(&client.from_addr);
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(0, nonce);

    let to_address = Address::from_str("0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71").unwrap();
    let token_id = config_gas_token_id();

    let eth_addr = EthereumAddress::new("0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71");
    let xx = MultiAddress::Vm(eth_addr);

    let bank_amount = test_rollup
        .client
        .get_balance::<DemoRollupSpec>(&xx, &token_id, None)
        .await
        .unwrap();

    let b1 = client.eth_get_balance(to_address).await;
    println!("{:?} {:?}", b1, bank_amount);
    client.send_eth(to_address, 2).await;

    sleep(Duration::from_secs(1)).await;

    let b1 = client.eth_get_balance(to_address).await;

    let bank_amount = test_rollup
        .client
        .get_balance::<DemoRollupSpec>(&xx, &token_id, None)
        .await
        .unwrap();

    println!("{:?} {:?}", b1, bank_amount);

    Ok(())
}
