use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::setup_with_simple_storage;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter};
use sov_demo_rollup::MockDemoRollup;
use sov_eth_client::SimpleStorageClient;
use sov_ethereum::Cursor;
use sov_modules_api::execution_mode::Native;
use sov_rpc_eth_types::FilterWithCursor;
use sov_rpc_eth_types::LogsWithMaybeCursor;
use sov_sequencer::SeqConfigExtension;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::SimpleStorage;

#[tokio::test(flavor = "multi_thread")]
async fn get_log_from_pending_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;
    let client = alloy_client(rollup.http_addr);
    let contract = SimpleStorage::deploy(client.clone()).await?;
    let tx = contract.emitLogs(U256::ZERO, U256::from(1)).send().await?;
    rollup.pause_preferred_batches().await;
    let receipt = tx.get_receipt().await?;

    let filter = Filter::new()
        .from_block(BlockNumberOrTag::Pending)
        .to_block(BlockNumberOrTag::Pending);
    let logs = client.get_logs(&filter).await?;
    let receipt_logs = receipt.inner.into_logs();
    assert_eq!(receipt_logs, logs);
    assert_eq!(receipt_logs.len(), 1);
    assert_eq!(receipt_logs[0].block_hash, None);
    assert_eq!(receipt_logs[0].block_timestamp, Some(0));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new(EVM_EXTENSION.max_log_limit).await;

    // Make sure all the txs are in the same blcok.
    rollup_and_client
        .test_rollup
        .pause_preferred_batches()
        .await;

    let tx_hashes = rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;

    rollup_and_client
        .test_rollup
        .resume_preferred_batches()
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let tx_hash = tx_hashes[0];
    let rec = rollup_and_client
        .client
        .alloy_receipt(tx_hash)
        .await
        .unwrap();
    let block_hash = rec.block_hash.unwrap();

    {
        let filter = Filter::new().at_block_hash(block_hash);
        let logs = rollup_and_client.client.get_logs(&filter).await;
        assert_eq!(logs.len() as u32, nb_of_txs * nb_of_logs_per_tx);

        for (index, log) in logs.into_iter().enumerate() {
            let index = index as u64;
            assert!(filter.matches(log.inner.as_ref()));
            assert_eq!(log.log_index.unwrap(), index);
            assert_eq!(
                log.transaction_index.unwrap(),
                (index / nb_of_logs_per_tx as u64)
            );
        }
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new().at_block_hash(block_hash).topic3(topic);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range() {
    let nb_of_txs = 10;
    let nb_of_logs_per_tx = 5;

    let rollup_and_client = RollupAndClient::new(EVM_EXTENSION.max_log_limit).await;

    let start_block = rollup_and_client
        .client
        .alloy_get_block_by_number(Some(BlockNumberOrTag::Latest.to_string()))
        .await
        .number();

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    // Check logs from all txs.
    {
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs * nb_of_logs_per_tx);
    }

    // topic3 seolects one log from each tx
    {
        let topic: B256 = U256::from(3).into();
        let filter = Filter::new()
            .from_block(start_block)
            .to_block(BlockNumberOrTag::Latest)
            .topic3(topic);

        let logs = rollup_and_client.client.get_logs(&filter).await;
        check_logs(&filter, logs, nb_of_txs);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_range_limit() {
    let max_log_limit = 93;
    let nb_of_txs = 20;
    let nb_of_logs_per_tx: u32 = 5;

    let rollup_and_client = RollupAndClient::new(max_log_limit).await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let filter = new_filter_for_all_logs();
    let logs = rollup_and_client.client.get_logs(&filter).await;

    assert_eq!(logs.len(), max_log_limit);
}

fn check_logs(filter: &Filter, logs: Vec<alloy_rpc_types_eth::Log>, expected_nb_of_logs: u32) {
    assert_eq!(logs.len() as u32, expected_nb_of_logs);
    for log in logs {
        assert!(filter.matches(log.inner.as_ref()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor_and_filter() {
    let max_log_limit = 7;
    let nb_of_txs = 9;
    let nb_of_logs_per_tx: u32 = 12;

    let rollup_and_client = RollupAndClient::new(max_log_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor_and_filter(&serde_json::json!({
            "fromBlock": "0x0",
            "toBlock": "latest",
    })).await;

    nb_of_logs_received += logs_with_cursor.logs.len();

    let mut nb_of_logs_until_prev_cursor = start_tx * nb_of_logs_per_tx;
    loop {
        let packed_cursor = match logs_with_cursor.cursor {
            Some(packed) => packed,
            None => {
                let total = (nb_of_txs * nb_of_logs_per_tx) as usize;
                assert_eq!(logs_with_cursor.logs.len(), total % max_log_limit);
                break;
            }
        };

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        let nb_of_logs_according_to_cursor =
            nb_of_logs_according_to_cursor(cursor, nb_of_logs_per_tx);

        // Every cursor gives max_log_limit logs.
        assert_eq!(
            nb_of_logs_according_to_cursor - nb_of_logs_until_prev_cursor,
            max_log_limit as u32
        );

        nb_of_logs_until_prev_cursor = nb_of_logs_according_to_cursor;

        logs_with_cursor = rollup_and_client.get_logs_with_cursor_and_filter(&serde_json::json!({
            "fromBlock": "0x0",
            "toBlock": "latest",
            "cursor": packed_cursor,
        })).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
}



#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_with_cursor() {
    let max_log_limit = 7;
    let nb_of_txs = 9;
    let nb_of_logs_per_tx: u32 = 2;

    let rollup_and_client = RollupAndClient::new(max_log_limit).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();

    let mut nb_of_logs_until_prev_cursor = start_tx * nb_of_logs_per_tx;
    loop {
        let packed_cursor = match logs_with_cursor.cursor {
            Some(packed) => packed,
            None => {
                let total = (nb_of_txs * nb_of_logs_per_tx) as usize;
                assert_eq!(logs_with_cursor.logs.len(), total % max_log_limit);
                break;
            }
        };

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        let nb_of_logs_according_to_cursor =
            nb_of_logs_according_to_cursor(cursor, nb_of_logs_per_tx);

        // Every cursor gives max_log_limit logs.
        assert_eq!(
            nb_of_logs_according_to_cursor - nb_of_logs_until_prev_cursor,
            max_log_limit as u32
        );

        nb_of_logs_until_prev_cursor = nb_of_logs_according_to_cursor;

        logs_with_cursor = rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
}


#[tokio::test(flavor = "multi_thread")]
async fn evm_test_get_logs_at_max_response_size() {
    let nb_of_txs = 3;
    let nb_of_logs_per_tx: u32 = 5000;

    let rollup_and_client = RollupAndClient::new(20_000).await;
    let start_tx = rollup_and_client.get_tx_count().await as u32;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, Some(3))
        .await;

    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let mut nb_of_logs_received = 0;
    let mut logs_with_cursor = rollup_and_client.get_logs_with_cursor(None).await;

    nb_of_logs_received += logs_with_cursor.logs.len();

    let mut nb_of_logs_until_prev_cursor = start_tx * nb_of_logs_per_tx;
    loop {
        let packed_cursor = match logs_with_cursor.cursor {
            Some(packed) => packed,
            None => {
                break;
            }
        };

        let cursor = Cursor::unpack(&packed_cursor).unwrap();
        let nb_of_logs_according_to_cursor =
            nb_of_logs_according_to_cursor(cursor, nb_of_logs_per_tx);

        nb_of_logs_until_prev_cursor = nb_of_logs_according_to_cursor;

        logs_with_cursor = rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
        nb_of_logs_received += logs_with_cursor.logs.len();
    }

    assert_eq!(nb_of_logs_received as u32, nb_of_txs * nb_of_logs_per_tx);
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_resumed_from_the_middle_of_tx_have_correct_indices() {
    let max_log_limit = 1;
    let nb_of_txs = 1;
    let nb_of_logs_per_tx: u32 = 2;

    let rollup_and_client = RollupAndClient::new(max_log_limit).await;

    rollup_and_client
        .produce_logs(nb_of_txs, nb_of_logs_per_tx, None)
        .await;
    rollup_and_client.test_rollup.wait_for_next_blocks(1).await;

    let LogsWithMaybeCursor { mut logs, cursor } =
        rollup_and_client.get_logs_with_cursor(None).await;
    let cursor = cursor.map(|c| Cursor::unpack(&c).unwrap()).unwrap();
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    assert_eq!(log.transaction_index, Some(0));
    assert_eq!(log.log_index, Some(0));
    assert_eq!(cursor.tx_index_absolute, 1);
    assert_eq!(cursor.log_index_in_tx, 1);

    let LogsWithMaybeCursor { mut logs, .. } =
        rollup_and_client.get_logs_with_cursor(Some(cursor)).await;
    assert_eq!(logs.len(), 1);
    let log = logs.pop().unwrap();
    assert_eq!(log.transaction_index, Some(0));
    assert_eq!(log.log_index, Some(1));
}

fn nb_of_logs_according_to_cursor(cursor: Cursor, nb_of_logs_per_tx: u32) -> u32 {
    (cursor.tx_index_absolute as u32) * nb_of_logs_per_tx + cursor.log_index_in_tx
}

fn new_filter_for_all_logs() -> Filter {
    Filter::new()
        .from_block(0)
        .to_block(BlockNumberOrTag::Latest)
}

struct RollupAndClient {
    test_rollup: TestRollup<MockDemoRollup<Native>>,
    client: SimpleStorageClient,
    contract_address: alloy_primitives::Address,
}

impl RollupAndClient {
    async fn new(max_log_limit: usize) -> RollupAndClient {
        let ext = SeqConfigExtension { max_log_limit };

        let (test_rollup, evm_client, _) = setup_with_simple_storage(0, ext).await;
        let contract_address = evm_client.alloy_deploy_contract().await;

        RollupAndClient {
            test_rollup,
            client: evm_client,
            contract_address,
        }
    }

    async fn produce_logs(
        &self,
        nb_of_txs: u32,
        nb_of_logs_per_tx: u32,
        block_interval: Option<u32>,
    ) -> Vec<alloy_primitives::TxHash> {
        let mut tx_hashes = Vec::with_capacity(nb_of_txs as usize);
        for i in 0..nb_of_txs {
            let hash = self
                .client
                .alloy_emit_logs(self.contract_address, i, nb_of_logs_per_tx)
                .await;
            tx_hashes.push(hash);
            if let Some(block_interval) = block_interval {
                if i % block_interval == 0 {
                    self.test_rollup.wait_for_next_blocks(1).await;
                }
            }
        }

        tx_hashes
    }

    async fn get_tx_count(&self) -> u64 {
        self.client
            .eth_get_transaction_count(self.client.address())
            .await
    }

    async fn get_logs_with_cursor(&self, cursor: Option<Cursor>) -> LogsWithMaybeCursor {
        let filter_with_cursor = FilterWithCursor {
            cursor: cursor.map(|c| c.pack()),
            filter: new_filter_for_all_logs(),
        };

        self.client.get_logs_with_cursor(&filter_with_cursor).await
    }

    async fn get_logs_with_cursor_and_filter(&self, cursor_and_filter: &impl serde::Serialize) -> LogsWithMaybeCursor {
        self.client.get_logs_with_cursor_and_filter(cursor_and_filter).await
    }
}
