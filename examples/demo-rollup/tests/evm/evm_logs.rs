use super::evm_test_helper;
use crate::evm::evm_test_helper::setup;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_logs() {
    let (test_rollup, evm_client, _, _) = setup(0).await;
    let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
        .await
        .unwrap();

    test_rollup.wait_for_next_blocks(1).await;

    let set_arg = 1;
    let pending_log = evm_client.set_value(contract_address, set_arg).await;
    let tx_hash = pending_log.tx_hash();

    test_rollup.wait_for_next_blocks(1).await;
    let rec = evm_client.receipt(tx_hash).await.unwrap();
    let log = rec.logs.first().unwrap();

    assert_eq!(log.transaction_hash.unwrap(), tx_hash);
    assert_eq!(log.address, contract_address);

    let value_from_logs = log.data.0.slice(28..32);
    assert_eq!(*value_from_logs, set_arg.to_be_bytes());
}
