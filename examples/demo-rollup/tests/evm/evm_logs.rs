use super::evm_test_helper;
use crate::evm::evm_test_helper::setup;
use ethers::contract::EthEvent;
use ethers::core::types::Address;
use ethers::core::types::U256;
use sov_test_utils::SimpleStorageContract;

#[derive(Debug, Clone, EthEvent)]
#[ethevent(name = "SimpleLog", abi = "Transfer(address,uint256)")]
struct SimpleLog {
    #[ethevent(indexed)]
    pub address: Address,
    pub value: U256,
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_logs() {
    let (test_rollup, evm_client, _) = setup(0).await;

    /*  let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
            .await
            .unwrap();

        test_rollup.wait_for_next_blocks(1).await;

        let set_arg = 1;
        let pending_log = evm_client.set_value(contract_address, set_arg).await;
        let tx_hash = pending_log.tx_hash();

        test_rollup.wait_for_next_blocks(1).await;
        let rec = evm_client.receipt(tx_hash).await.unwrap();
        let log = rec.logs.first().unwrap();

        let contract_log = SimpleStorageContract::parse_simple_log(log.clone());

        assert_eq!(contract_log.original.transaction_hash.unwrap(), tx_hash);
        assert_eq!(contract_log.original.address, contract_address);

        assert_eq!(contract_log.paresed.value, set_arg.into());
    */
    let mut sub = evm_client.subscribe_logs().await;
    loop {
        let log = sub.recv().await.unwrap();
        println!("{:?}", log);
    }
}
