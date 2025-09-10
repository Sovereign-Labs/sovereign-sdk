use std::hash::Hash;

use super::evm_test_helper;
use crate::evm::evm_test_helper::setup;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_logs() {
    let (test_rollup, evm_client, _, _) = setup(0).await;
    let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
        .await
        .unwrap();

    test_rollup.wait_for_next_blocks(1).await;

    let pending_log = evm_client.emit_one_log(contract_address).await;
    let tx_hash_1 = pending_log.tx_hash();
    test_rollup.wait_for_next_blocks(1).await;
    let rec = evm_client.receipt(tx_hash_1).await.unwrap();
    let log = rec.logs.first().unwrap();

    assert_eq!(log.address, contract_address);
    assert_eq!(log.transaction_hash, tx_hash_1);
    //log.data

    //println!("logs {:?}", rec.logs);
}
