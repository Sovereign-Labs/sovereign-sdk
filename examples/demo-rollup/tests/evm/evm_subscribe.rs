use std::ops::Range;
use std::sync::Arc;

use crate::evm::evm_test_helper::setup;
use alloy_primitives::Address;
use alloy_primitives::TxHash;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_eth::Filter;
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::TestClient;
use sov_modules_api::execution_mode::Native;
use sov_test_utils::test_rollup::TestRollup;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio::time::Duration;

// The basic subscription test.
#[tokio::test(flavor = "multi_thread")]
async fn evm_test_log_subscription() {
    let (test_rollup, evm_client, _) = setup(0).await;
    let mut log_collector = LogCollector::new();

    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;

    // Logs from this transactions should not appear in the subscription because we haven't subscribed yet.
    send_txs_and_pause(0..10, contract_address, &evm_client, &test_rollup).await;

    let sub = evm_client.alloy_subscribe_logs(&Filter::new()).await;
    let sub_id = sub.local_id().clone();

    // Subscription started. Logs from these transactions will appear even though we haven’t started listening for them.
    let mut tx_hashes =
        send_txs_and_pause(10..15, contract_address, &evm_client, &test_rollup).await;

    // Log listening started.
    log_collector.spawn_log_watcher(sub, None).await;

    // Logs from this txs will shouw up in the subscription.
    let mut tx_hashes_2 =
        send_txs_and_pause(15..100, contract_address, &evm_client, &test_rollup).await;
    tx_hashes.append(&mut tx_hashes_2);

    let logs_fetched = fetch_logs(tx_hashes, &evm_client).await;

    // Kill subscription.
    evm_client.alloy_unsubscribe(sub_id);
    log_collector.wait().await;

    let logs_from_subscription = log_collector.logs().await;

    assert_eq!(logs_from_subscription.len(), logs_fetched.len());

    let mut block_nr = 0;
    let mut time_stamp = 0;
    for (i, log) in logs_fetched.iter().enumerate() {
        let sub_log = &logs_from_subscription[i];
        let block_nr_from_log = log.block_number.unwrap();

        // Verify that the block timestamp increases along with the block number.
        if block_nr_from_log > block_nr {
            let block_timestamp_from_log = log.block_timestamp.unwrap();
            assert!(block_timestamp_from_log > time_stamp);
            time_stamp = block_timestamp_from_log;
            block_nr = block_nr_from_log;
        }

        assert_logs(log, sub_log);
    }

    assert!(block_nr > 0);
}

// Tests for logs from pending block.
#[tokio::test(flavor = "multi_thread")]
async fn evm_test_log_subscription_with_pending_blcok() {
    let (test_rollup, evm_client, _) = setup(0).await;
    let mut log_collector = LogCollector::new();

    let contract_address = evm_client.alloy_deploy_contract().await;
    test_rollup.wait_for_next_blocks(1).await;
    test_rollup.pause_preferred_batches().await;

    let nb_of_txs = 100;

    let sub = evm_client.alloy_subscribe_logs(&Filter::new()).await;

    log_collector
        .spawn_log_watcher(sub, Some(nb_of_txs as usize))
        .await;

    let mut tx_hashes = Vec::new();

    for i in 0..nb_of_txs {
        let hash = evm_client.alloy_set_value(contract_address, i).await;
        tx_hashes.push(hash);
    }

    log_collector.wait().await;

    let logs_from_subscription = log_collector.logs().await;
    assert_eq!(logs_from_subscription.len(), nb_of_txs as usize);

    let block_nr = evm_client
        .eth_get_block_by_number(Some("pending".to_string()))
        .await
        .number
        .unwrap()
        .as_u64();

    // Verify conditions for logs in the pending block.
    for log in logs_from_subscription {
        assert!(log.block_hash.is_none());
        assert_eq!(log.block_number.unwrap(), block_nr);
        assert!(log.block_timestamp.unwrap() > 0);
    }
}

// Subscription test with filtering.
// `SimpleStorageContract::set_value` emits logs where topic2 matches the function argument,
// and topic3 ranges from 0 to nb_of_logs_per_tx.
// This behavior is used to verify the filtering mechanism.
#[tokio::test(flavor = "multi_thread")]
async fn evm_test_logs_filter() {
    TestCase {
        nb_of_txs: 17,
        nb_of_logs_per_tx: 5,
        raw_topic2: None,
        raw_topic3: None,
    }
    .run()
    .await;

    TestCase {
        nb_of_txs: 17,
        nb_of_logs_per_tx: 5,
        raw_topic2: Some(2),
        raw_topic3: None,
    }
    .run()
    .await;

    TestCase {
        nb_of_txs: 17,
        nb_of_logs_per_tx: 5,
        raw_topic2: None,
        raw_topic3: Some(4),
    }
    .run()
    .await;

    TestCase {
        nb_of_txs: 17,
        nb_of_logs_per_tx: 5,
        raw_topic2: Some(12),
        raw_topic3: Some(4),
    }
    .run()
    .await;
}

struct TestCase {
    nb_of_txs: u32,
    nb_of_logs_per_tx: u32,
    raw_topic2: Option<u32>,
    raw_topic3: Option<u32>,
}

impl TestCase {
    fn filter(&self) -> Filter {
        let mut filter = Filter::new();

        if let Some(topic2) = self.raw_topic2 {
            let topic: B256 = U256::from(topic2).into();
            filter = filter.topic2(topic);
        }

        if let Some(topic3) = self.raw_topic3 {
            let topic: B256 = U256::from(topic3).into();
            filter = filter.topic3(topic);
        }

        filter
    }

    async fn run(&self) {
        let (test_rollup, evm_client, _) = setup(0).await;
        let contract_address = evm_client.alloy_deploy_contract().await;
        test_rollup.wait_for_next_blocks(1).await;

        let filter = self.filter();

        let logs_from_subscription = get_filtered_logs(
            self.nb_of_txs,
            self.nb_of_logs_per_tx,
            &filter,
            contract_address,
            &evm_client,
            &test_rollup,
        )
        .await;

        // Topic2 selects a single tx, and Topic3 selects a single log from that tx.
        let mut nb_of_txs = self.nb_of_txs;
        if self.raw_topic2.is_some() {
            nb_of_txs = 1;
        }

        let mut nb_of_logs_per_tx = self.nb_of_logs_per_tx;
        if self.raw_topic3.is_some() {
            nb_of_logs_per_tx = 1;
        }

        let expected_nb_of_logs = nb_of_txs * nb_of_logs_per_tx;

        assert_eq!(logs_from_subscription.len(), expected_nb_of_logs as usize);

        for log in logs_from_subscription {
            assert!(filter.matches(&log.inner));
        }
    }
}

struct LogCollector {
    logs_from_subscription: Arc<Mutex<Vec<alloy_rpc_types_eth::Log>>>,
    snd: tokio::sync::watch::Sender<()>,
    rec: tokio::sync::watch::Receiver<()>,
}

impl LogCollector {
    fn new() -> Self {
        let (snd, rec) = tokio::sync::watch::channel(());

        Self {
            logs_from_subscription: Arc::new(Mutex::new(Vec::new())),
            snd,
            rec,
        }
    }

    async fn spawn_log_watcher(
        &mut self,
        mut sub: alloy_pubsub::Subscription<alloy_rpc_types_eth::Log>,
        expected_nr_of_logs: Option<usize>,
    ) {
        let logs_from_subscription = self.logs_from_subscription.clone();
        let snd = self.snd.clone();

        tokio::spawn(async move {
            loop {
                if let Some(expected_nr_of_logs) = expected_nr_of_logs {
                    let len = logs_from_subscription.lock().await.len();

                    if len >= expected_nr_of_logs {
                        snd.send(()).unwrap();
                        return;
                    }
                }

                match sub.recv().await {
                    Ok(log) => {
                        logs_from_subscription.lock().await.push(log);
                    }
                    Err(e) => match e {
                        broadcast::error::RecvError::Closed => {
                            snd.send(()).unwrap();
                            break;
                        }
                        broadcast::error::RecvError::Lagged(err) => {
                            panic!("Log watcher error: {:?}", err)
                        }
                    },
                }
            }
        });
    }

    async fn logs(&self) -> Vec<alloy_rpc_types_eth::Log> {
        self.logs_from_subscription.lock().await.clone()
    }

    async fn wait(&mut self) {
        self.rec.changed().await.unwrap()
    }
}

async fn send_txs_and_pause(
    range: Range<u32>,
    contract_address: Address,
    evm_client: &TestClient,
    test_rollup: &TestRollup<MockDemoRollup<Native>>,
) -> Vec<TxHash> {
    let mut tx_hashes = Vec::new();

    for i in range {
        let set_arg = i;
        let hash = evm_client.alloy_set_value(contract_address, set_arg).await;
        if i % 3 == 0 {
            test_rollup.wait_for_next_blocks(1).await;
        }
        tx_hashes.push(hash);
    }

    tx_hashes
}

async fn fetch_logs(
    tx_hashes: Vec<TxHash>,
    evm_client: &TestClient,
) -> Vec<alloy_rpc_types_eth::Log> {
    let mut logs = Vec::new();

    for hash in &tx_hashes {
        let receipt = evm_client.alloy_receipt(hash.clone()).await.unwrap();
        for log in receipt.logs() {
            assert_receipt_and_log(&receipt, log);
            logs.push(log.clone());
        }
    }
    logs
}

async fn get_filtered_logs(
    number_of_txs: u32,
    nb_of_logs: u32,
    filter: &Filter,
    contract_address: Address,
    evm_client: &TestClient,
    test_rollup: &TestRollup<MockDemoRollup<Native>>,
) -> Vec<alloy_rpc_types_eth::Log> {
    let mut log_collector = LogCollector::new();
    let sub = evm_client.alloy_subscribe_logs(&filter).await;
    let sub_id = sub.local_id().clone();

    log_collector.spawn_log_watcher(sub, None).await;

    for i in 0..number_of_txs {
        let _ = evm_client
            .alloy_emit_logs(contract_address, i as u32, nb_of_logs)
            .await;

        if i % 5 == 0 {
            test_rollup.wait_for_next_blocks(1).await;
        }
    }

    tokio::time::sleep(Duration::from_secs(2)).await;

    evm_client.alloy_unsubscribe(sub_id);
    log_collector.wait().await;
    log_collector.logs().await
}

fn assert_logs(first: &alloy_rpc_types_eth::Log, second: &alloy_rpc_types_eth::Log) {
    assert_eq!(first.block_number, second.block_number);
    assert_eq!(first.inner, second.inner);
    assert_eq!(first.transaction_hash, second.transaction_hash);
    assert_eq!(first.transaction_index, second.transaction_index);
    assert_eq!(first.log_index, second.log_index);
    assert_eq!(first.removed, second.removed);
}

fn assert_receipt_and_log(
    receipt: &alloy_rpc_types_eth::TransactionReceipt,
    log: &alloy_rpc_types_eth::Log,
) {
    assert_eq!(receipt.block_hash, log.block_hash);
    assert_eq!(receipt.block_number, log.block_number);
    assert_eq!(receipt.transaction_hash, log.transaction_hash.unwrap());
    assert_eq!(receipt.transaction_index, log.transaction_index);
}
