use alloy_primitives::{Address, U256};
use sov_eth_client::SimpleStorageClient;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::transaction::{Transaction, UnsignedTransactionV0};
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::{TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE};

use crate::common::{
    deploy_contract_check, set_value_check, setup_with_simple_storage, EVM_EXTENSION,
};
use crate::runtime::{EvmTestSpec, TestRuntime, TestRuntimeCall};

type TestSpec = EvmTestSpec;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Account abstraction for the EVM is disabled"]
async fn test_evm_account_abstraction() {
    let (test_rollup, test_client, chain_id) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    // Before executing the evm checks we need to insert the credentials in the `Accounts`.
    send_insert_credentials(&test_client, test_client.address(), chain_id).await;
    // Execute the evm tests.
    execute_evm_tests(&test_client).await.unwrap();

    test_rollup.rollup_task.abort();
}

async fn send_insert_credentials(
    test_client: &SimpleStorageClient,
    from_addr: Address,
    chain_id: u64,
) {
    let tx = create_insert_credentials(from_addr, chain_id);
    test_client
        .send_transaction_and_wait_slot(&tx)
        .await
        .unwrap();
}

fn create_insert_credentials(
    from_addr: Address,
    chain_id: u64,
) -> Transaction<TestRuntime<TestSpec>, TestSpec> {
    let nonce = 0;
    let key_and_address = read_private_key::<TestSpec>("tx_signer_private_key.json");
    let key = key_and_address.private_key;

    let mut credentials = [0; 32];
    credentials[12..].copy_from_slice(&from_addr.0 .0);

    let msg = TestRuntimeCall::<TestSpec>::Accounts(sov_accounts::CallMessage::InsertCredentialId(
        credentials.into(),
    ));

    let max_priority_fee_bips = TEST_DEFAULT_MAX_PRIORITY_FEE;
    let max_fee = TEST_DEFAULT_MAX_FEE;
    let gas_limit = None;
    let chain_hash =
        <TestRuntime<TestSpec> as sov_modules_stf_blueprint::Runtime<TestSpec>>::CHAIN_HASH;
    Transaction::<TestRuntime<TestSpec>, TestSpec>::new_signed_tx(
        &key,
        &chain_hash,
        UnsignedTransactionV0::new(
            msg,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            UniquenessData::Nonce(nonce),
            gas_limit,
        ),
    )
}

async fn execute_evm_tests(client: &SimpleStorageClient) -> Result<(), Box<dyn std::error::Error>> {
    let nonce = client.eth_get_transaction_count(client.address()).await;
    assert_eq!(0, nonce);

    // Balance should be > 0 in genesis
    let balance = client.eth_get_balance(client.address()).await;
    assert!(balance > U256::ZERO);

    let contract_address = deploy_contract_check(client).await?;

    // Nonce should be 1 after the deploy
    let nonce = client.eth_get_transaction_count(client.address()).await;
    assert_eq!(1, nonce);

    let set_arg = 923;
    set_value_check(client, contract_address, set_arg).await?;

    Ok(())
}
