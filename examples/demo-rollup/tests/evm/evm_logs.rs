use crate::evm::evm_test_helper::setup;
use sov_test_utils::SimpleStorageContract;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_logs() {
    let (test_rollup, evm_client, _) = setup(0).await;
    let mut sub = evm_client.alloy_subscribe_logs().await;

    tokio::spawn(async move {
        loop {
            println!("Start");
            let log = sub.recv().await;
            println!("XXXXXXXXX {:?}", log);
            if log.is_err() {
                break;
            }
        }
    });

    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    for i in 1..3 {
        let set_arg = i;
        let tx_hash = evm_client.alloy_set_value(contract_address, set_arg).await;

        test_rollup.wait_for_next_blocks(1).await;
        let rec = evm_client.alloy_receipt(tx_hash).await.unwrap();
        let log = rec.inner.logs().first().unwrap();
        let contract_log = SimpleStorageContract::decode_alloy(log.clone());
        println!("Log {:?}", contract_log);
        println!("");

        assert_eq!(contract_log.original.transaction_hash.unwrap(), tx_hash);
        assert_eq!(contract_log.original.address(), contract_address);

        assert_eq!(
            contract_log.paresed.value,
            alloy_primitives::U256::from(set_arg)
        );
    }
}
