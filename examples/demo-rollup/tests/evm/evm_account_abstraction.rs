use demo_stf::runtime::{Runtime, RuntimeCall};
use ethers_core::abi::Address;
use sov_demo_rollup::{mock_da_risc0_host_args, MockRollupSpec};
use sov_eth_client::TestClient;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_macros::config_value;
use sov_test_utils::test_rollup::{get_appropriate_rollup_prover_config, read_private_key};
use sov_test_utils::{TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE};

use crate::evm::evm_test_helper::{self};
use crate::test_helpers::{DemoRollupSpec, CHAIN_HASH};

type TestSpec = DemoRollupSpec;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "This test is currently broken. We need to integrate the EVM module with the soft-confirmation kernel to make it work again. Relevant issue: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1904"]
async fn test_evm_account_abstraction() {
    let chain_id = config_value!("CHAIN_ID");
    let finalization_blocks = 0;
    let rollup_prover_config =
        get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(mock_da_risc0_host_args());
    // tempdir is held here so it is not removed during test run
    let test_rollup = evm_test_helper::start_node(rollup_prover_config, finalization_blocks).await;

    let (test_client, from_addr) = evm_test_helper::create_test_client(
        test_rollup.http_addr,
        chain_id,
        // This will produce an evm key that doesn't exist in rollup accounts-genesis so we have to register the credentials in the rollup.
        "0x90cb5be9e2c125d84af44f19a4e6e36af359bd47b41577aedbe8aa24313bbd40",
    )
    .await;

    // Before executing the evm checks we need to insert the credentials in the `Accounts`.
    send_insert_credentials(&test_client, from_addr, chain_id).await;
    // Execute the evm tests.
    execute_evm_tests(&test_client).await.unwrap();

    test_rollup.rollup_task.abort();
}

async fn send_insert_credentials(test_client: &TestClient, from_addr: Address, chain_id: u64) {
    let tx = vec![create_insert_credentials(from_addr, chain_id)];
    test_client
        .send_transactions_and_wait_slot(&tx)
        .await
        .unwrap();
}

fn create_insert_credentials(
    from_addr: Address,
    chain_id: u64,
) -> Transaction<Runtime<TestSpec>, TestSpec> {
    let nonce = 0;
    let key_and_address = read_private_key::<TestSpec>("tx_signer_private_key.json");
    let key = key_and_address.private_key;

    let mut credentials = [0; 32];
    credentials[12..].copy_from_slice(&from_addr.to_fixed_bytes());

    let msg = RuntimeCall::<TestSpec>::Accounts(sov_accounts::CallMessage::InsertCredentialId(
        credentials.into(),
    ));

    let max_priority_fee_bips = TEST_DEFAULT_MAX_PRIORITY_FEE;
    let max_fee = TEST_DEFAULT_MAX_FEE;
    let gas_limit = None;
    Transaction::<Runtime<TestSpec>, TestSpec>::new_signed_tx(
        &key,
        &CHAIN_HASH,
        UnsignedTransaction::new(
            msg,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            nonce,
            gas_limit,
        ),
    )
}

async fn execute_evm_tests(client: &TestClient) -> Result<(), Box<dyn std::error::Error>> {
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(0, nonce);

    // Balance should be > 0 in genesis
    let balance = client.eth_get_balance(client.from_addr).await;
    assert!(balance > ethereum_types::U256::zero());

    let contract_address = evm_test_helper::deploy_contract_check(client).await?;

    // Nonce should be 1 after the deploy
    let nonce = client.eth_get_transaction_count(client.from_addr).await;
    assert_eq!(1, nonce);

    let set_arg = 923;
    evm_test_helper::set_value_check(client, contract_address, set_arg).await?;

    Ok(())
}
