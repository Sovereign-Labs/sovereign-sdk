use ethereum_types::H160;
use sov_eth_client::TestClient;

use crate::evm::evm_test_helper;
use crate::evm::evm_test_helper::setup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_crate() {
    let (_, evm_client, _) = setup(0, EVM_EXTENSION).await;

    let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
        .await
        .unwrap();

    let inner_addr_1 = get_inner_contract_addr(contract_address, &evm_client).await;
    let inner_addr_2 = get_inner_contract_addr(contract_address, &evm_client).await;

    assert_ne!(inner_addr_1, inner_addr_2)
}

async fn get_inner_contract_addr(contract_address: H160, evm_client: &TestClient) -> H160 {
    let receipt = evm_client
        .deploy_inner_contract(contract_address)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    receipt.contract_address.unwrap()
}
