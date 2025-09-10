use crate::evm::evm_test_helper::{self};
use crate::test_helpers::{DemoRollupSpec, CHAIN_HASH};
use demo_stf::runtime::{Runtime, RuntimeCall};
use ethers::core::abi::Address;
use sov_eth_client::TestClient;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::{TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE};

type TestSpec = DemoRollupSpec;
use crate::evm::evm_test_helper::setup;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Account abstraction for the EVM is disabled"]
async fn test_evm_account_abstraction() {
    let (test_rollup, test_client, from_addr, chain_id) = setup(0).await;

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
            UniquenessData::Nonce(nonce),
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
