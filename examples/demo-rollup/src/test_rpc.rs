use std::collections::HashMap;
use std::net::SocketAddr;

use proptest::prelude::any_with;
use proptest::strategy::Strategy;
use proptest::{prop_compose, proptest};
use reqwest::header::CONTENT_TYPE;
use serde_json::json;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
#[cfg(test)]
use sov_rollup_interface::mocks::{MockBlock, MockBlockHeader, MockHash};
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::stf::fuzzing::BatchReceiptStrategyArgs;
use sov_rollup_interface::stf::{BatchReceipt, Event, TransactionReceipt};
#[cfg(test)]
use sov_stf_runner::get_ledger_rpc;
use sov_stf_runner::RpcConfig;
use tendermint::crypto::Sha256;
use tokio::sync::oneshot;

struct TestExpect {
    payload: serde_json::Value,
    expected: serde_json::Value,
}

async fn queries_test_runner(test_queries: Vec<TestExpect>, rpc_config: RpcConfig) {
    let (addr, port) = (rpc_config.bind_host, rpc_config.bind_port);
    let client = reqwest::Client::new();
    let url_str = format!("http://{addr}:{port}");

    for query in test_queries {
        let res = client
            .post(url_str.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(query.payload.to_string())
            .send()
            .await
            .unwrap();

        assert_eq!(res.status().as_u16(), 200);

        let response_body = res.text().await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response_body).unwrap(),
            query.expected,
        );
    }
}

fn populate_ledger(ledger_db: &mut LedgerDB, slots: Vec<SlotCommit<MockBlock, u32, u32>>) {
    for slot in slots {
        ledger_db.commit_slot(slot).unwrap();
    }
}

fn test_helper(test_queries: Vec<TestExpect>, slots: Vec<SlotCommit<MockBlock, u32, u32>>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();

    rt.block_on(async {
        let (tx_start, rx_start) = oneshot::channel();
        let (tx_end, rx_end) = oneshot::channel();

        let address = SocketAddr::new("127.0.0.1".parse().unwrap(), 0);

        // Initialize the ledger database, which stores blocks, transactions, events, etc.
        let tmpdir = tempfile::tempdir().unwrap();
        let mut ledger_db = LedgerDB::with_path(tmpdir.path()).unwrap();

        populate_ledger(&mut ledger_db, slots);

        let ledger_rpc_module = get_ledger_rpc::<u32, u32>(ledger_db.clone());

        rt.spawn(async move {
            let server = jsonrpsee::server::ServerBuilder::default()
                .build([address].as_ref())
                .await
                .unwrap();
            let actual_address = server.local_addr().unwrap();
            let _server_handle = server.start(ledger_rpc_module).unwrap();
            tx_start.send(actual_address.port()).unwrap();
            rx_end.await.unwrap();
        });

        let bind_port = rx_start.await.unwrap();
        let rpc_config = RpcConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port,
        };

        queries_test_runner(test_queries, rpc_config).await;

        tx_end.send("drop server").unwrap();
    });
}

fn batch2_tx_receipts() -> Vec<TransactionReceipt<u32>> {
    (0..260u64)
        .map(|i| TransactionReceipt::<u32> {
            tx_hash: ::sha2::Sha256::digest(i.to_string()),
            body_to_save: Some(b"tx body".to_vec()),
            events: vec![],
            receipt: 0,
        })
        .collect()
}

fn regular_test_helper(payload: serde_json::Value, expected: &serde_json::Value) {
    let mut slots: Vec<SlotCommit<MockBlock, u32, u32>> = vec![SlotCommit::new(MockBlock {
        header: MockBlockHeader {
            prev_hash: sha2::Sha256::digest(b"prev_header").into(),
            hash: sha2::Sha256::digest(b"slot_data").into(),
        },
        height: 0,
        validity_cond: Default::default(),
        blobs: Default::default(),
    })];

    let batches = vec![
        BatchReceipt {
            batch_hash: ::sha2::Sha256::digest(b"batch_receipt"),
            tx_receipts: vec![
                TransactionReceipt::<u32> {
                    tx_hash: ::sha2::Sha256::digest(b"tx1"),
                    body_to_save: Some(b"tx1 body".to_vec()),
                    events: vec![],
                    receipt: 0,
                },
                TransactionReceipt::<u32> {
                    tx_hash: ::sha2::Sha256::digest(b"tx2"),
                    body_to_save: Some(b"tx2 body".to_vec()),
                    events: vec![
                        Event::new("event1_key", "event1_value"),
                        Event::new("event2_key", "event2_value"),
                    ],
                    receipt: 1,
                },
            ],
            inner: 0,
        },
        BatchReceipt {
            batch_hash: ::sha2::Sha256::digest(b"batch_receipt2"),
            tx_receipts: batch2_tx_receipts(),
            inner: 1,
        },
    ];

    for batch in batches {
        slots.get_mut(0).unwrap().add_batch(batch)
    }

    test_helper(
        vec![TestExpect {
            payload,
            expected: expected.clone(),
        }],
        slots,
    )
}

/// Concisely generate a [JSON-RPC 2.0](https://www.jsonrpc.org/specification)
/// request [`String`]. You must provide the method name and the parameters of
/// the request, using [`serde_json::json!`] syntax.
///
/// ```
/// let req: String = jsonrpc_req!("method", ["param1", "param2"]);
/// ```
macro_rules! jsonrpc_req {
    ($method:expr, $params:tt) => {
        ::serde_json::json!({
            "jsonrpc": "2.0",
            "method": $method,
            "params": $params,
            "id": 1
        })
    };
}

/// A counterpart to [`jsonrpc_req!`] which generates successful responses.
macro_rules! jsonrpc_result {
    ($result:tt) => {{
        ::serde_json::json!({
            "jsonrpc": "2.0",
            "result": $result,
            "id": 1
        })
    }};
}

// These tests reproduce the README workflow for the ledger_rpc, ie:
// - It creates and populate a simple ledger with a few transactions
// - It initializes the rpc server
// - It successively calls the different rpc methods registered and tests the answer
#[test]
fn test_get_head() {
    let payload = jsonrpc_req!("ledger_getHead", []);
    let expected = jsonrpc_result!({"number":1,"hash":"0xd1231a38586e68d0405dc55ae6775e219f29fff1f7e0c6410d0ac069201e550b","batch_range":{"start":1,"end":3}});

    regular_test_helper(payload, &expected);
}

#[test]
fn test_get_transactions_offset_first_batch() {
    // Tests for different types of argument
    let payload = jsonrpc_req!("ledger_getTransactions", [[{"batch_id": 1, "offset": 0}]]);
    let expected = jsonrpc_result!([{"hash":"0x709b55bd3da0f5a838125bd0ee20c5bfdd7caba173912d4281cae816b79a201b","event_range":{"start":1,"end":1},"body":[116,120,49,32,98,111,100,121],"custom_receipt":0}]);
    regular_test_helper(payload, &expected);

    // Tests for flattened args
    let payload = jsonrpc_req!("ledger_getTransactions", [1]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getTransactions", [[1]]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getTransactions", [[1], "Standard"]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getTransactions", [[1], "Compact"]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getTransactions", [[1], "Full"]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getTransactions", [[{ "batch_id": 1, "offset": 1}]]);
    let expected = jsonrpc_result!([{"hash":"0x27ca64c092a959c7edc525ed45e845b1de6a7590d173fd2fad9133c8a779a1e3","event_range":{"start":1,"end":3},"body":[116,120,50,32,98,111,100,121],"custom_receipt":1}]);
    regular_test_helper(payload, &expected);
}

#[test]
fn test_get_batches() {
    let payload = jsonrpc_req!("ledger_getBatches", [[2], "Standard"]);
    let expected = jsonrpc_result!([{
        "hash":"0xf85fe0cb36fdaeca571c896ed476b49bb3c8eff00d935293a8967e1e9a62071e",
        "tx_range":{"start":3,"end":263},
        "txs": batch2_tx_receipts().into_iter().map(|tx_receipt| format!("0x{}", hex::encode(tx_receipt.tx_hash) )).collect::<Vec<_>>(),
        "custom_receipt":1
    }]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getBatches", [[2]]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getBatches", [2]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getBatches", [[1], "Compact"]);
    let expected = jsonrpc_result!([{"hash":"0xb5515a80204963f7db40e98af11aedb49a394b1c7e3d8b5b7a33346b8627444f","tx_range":{"start":1,"end":3},"custom_receipt":0}]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getBatches", [[1], "Full"]);
    let expected = jsonrpc_result!([{"hash":"0xb5515a80204963f7db40e98af11aedb49a394b1c7e3d8b5b7a33346b8627444f","tx_range":{"start":1,"end":3},"txs":[{"hash":"0x709b55bd3da0f5a838125bd0ee20c5bfdd7caba173912d4281cae816b79a201b","event_range":{"start":1,"end":1},"body":[116,120,49,32,98,111,100,121],"custom_receipt":0},{"hash":"0x27ca64c092a959c7edc525ed45e845b1de6a7590d173fd2fad9133c8a779a1e3","event_range":{"start":1,"end":3},"body":[116,120,50,32,98,111,100,121],"custom_receipt":1}],"custom_receipt":0}]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getBatches", [[0], "Compact"]);
    let expected = jsonrpc_result!([null]);
    regular_test_helper(payload, &expected);
}

#[test]
fn test_get_events() {
    let payload = jsonrpc_req!("ledger_getEvents", [1]);
    let expected = jsonrpc_result!([{
        "key":[101,118,101,110,116,49,95,107,101,121],
        "value":[101,118,101,110,116,49,95,118,97,108,117,101]
    }]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getEvents", [2]);
    let expected = jsonrpc_result!([{
        "key":[101,118,101,110,116,50,95,107,101,121],
        "value":[101,118,101,110,116,50,95,118,97,108,117,101]
    }]);
    regular_test_helper(payload, &expected);

    let payload = jsonrpc_req!("ledger_getEvents", [3]);
    let expected = jsonrpc_result!([null]);
    regular_test_helper(payload, &expected);
}

fn batch_receipt_without_hasher() -> impl Strategy<Value = BatchReceipt<u32, u32>> {
    let mut args = BatchReceiptStrategyArgs {
        hasher: None,
        ..Default::default()
    };
    args.transaction_strategy_args.hasher = None;
    any_with::<BatchReceipt<u32, u32>>(args)
}

prop_compose! {
    fn arb_batches_and_slot_hash(max_batches : usize)
     (slot_hash in proptest::array::uniform32(0_u8..), batches in proptest::collection::vec(batch_receipt_without_hasher(), 1..max_batches)) ->
       (Vec<BatchReceipt<u32, u32>>, [u8;32]) {
        (batches, slot_hash)
    }
}

prop_compose! {
    fn arb_slots(max_slots: usize, max_batches: usize)
    (batches_and_hashes in proptest::collection::vec(arb_batches_and_slot_hash(max_batches), 1..max_slots)) -> (Vec<SlotCommit<MockBlock, u32, u32>>, HashMap<usize, (usize, usize)>, usize)
    {
        let mut slots = std::vec::Vec::with_capacity(max_slots);

        let mut total_num_batches = 1;

        let mut prev_hash = MockHash([0;32]);

        let mut curr_tx_id = 1;
        let mut curr_event_id = 1;

        let mut tx_id_to_event_range = HashMap::new();

        for (batches, hash) in batches_and_hashes{
            let mut new_slot = SlotCommit::new(MockBlock {
                header: MockBlockHeader {
                hash: hash.into(),
                    prev_hash,
                },
                height: 0,
                validity_cond: Default::default(),
                blobs: Default::default()
            });

            total_num_batches += batches.len();

            for batch in batches {
                for tx in &batch.tx_receipts{
                    tx_id_to_event_range.insert(curr_tx_id, (curr_event_id, curr_event_id + tx.events.len()));

                    curr_event_id += tx.events.len();
                    curr_tx_id += 1;
                }

                new_slot.add_batch(batch);
            }


            slots.push(new_slot);

            prev_hash = MockHash(hash);
        }

        (slots, tx_id_to_event_range, total_num_batches)
    }
}

fn full_tx_json(
    tx_id: usize,
    tx: &TransactionReceipt<u32>,
    tx_id_to_event_range: &HashMap<usize, (usize, usize)>,
) -> serde_json::Value {
    let (event_range_begin, event_range_end) = tx_id_to_event_range.get(&tx_id).unwrap();
    let tx_hash_hex = hex::encode(tx.tx_hash);
    match &tx.body_to_save {
        None => json!({
            "hash": format!("0x{tx_hash_hex}"),
            "event_range": {
                "start": event_range_begin,
                "end": event_range_end
            },
            "custom_receipt": tx.receipt,
        }),
        Some(body) => {
            json!({
                "hash": format!("0x{tx_hash_hex}"),
                "event_range": {
                    "start": event_range_begin,
                    "end": event_range_end
                },
                "body": body,
                "custom_receipt": tx.receipt,
            })
        }
    }
}

proptest!(
    // Reduce the cases from 256 to 100 to speed up these tests
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]
    #[test]
    fn proptest_get_head((slots, _, total_num_batches) in arb_slots(10, 10)){
        let last_slot = slots.last().unwrap();
        let last_slot_num_batches = last_slot.batch_receipts().len();

        let last_slot_start_batch = total_num_batches - last_slot_num_batches;
        let last_slot_end_batch = total_num_batches;

        let payload = jsonrpc_req!("ledger_getHead", []);
        let expected = jsonrpc_result!({
            "number": slots.len(),
            "hash": format!("0x{}", hex::encode(last_slot.slot_data().hash())),
            "batch_range": {
                "start": last_slot_start_batch,
                "end": last_slot_end_batch
            }
        });
        test_helper(vec![TestExpect{ payload, expected }], slots);
    }


    #[test]
    fn proptest_get_batches((slots, tx_id_to_event_range, _total_num_batches) in arb_slots(10, 10), random_batch_num in 1..100){
        let mut curr_batch_num = 1;
        let mut curr_tx_num = 1;

        let random_batch_num_usize = usize::try_from(random_batch_num).unwrap();

        for slot in &slots {
            if curr_batch_num > random_batch_num_usize {
                break;
            }

            if curr_batch_num + slot.batch_receipts().len() > random_batch_num_usize {
                let curr_slot_batches = slot.batch_receipts();

                let batch_index = random_batch_num_usize - curr_batch_num;

                for i in 0..batch_index{
                    curr_tx_num += curr_slot_batches.get(i).unwrap().tx_receipts.len();
                }

                let first_tx_num = curr_tx_num;

                let curr_batch = curr_slot_batches.get(batch_index).unwrap();
                let last_tx_num = first_tx_num + curr_batch.tx_receipts.len();

                let batch_hash = hex::encode(curr_batch.batch_hash);
                let batch_receipt= curr_batch.inner;

                let tx_hashes: Vec<String> = curr_batch.tx_receipts.iter().map(|tx| {
                    format!("0x{}", hex::encode(tx.tx_hash))
                }).collect();

                let full_txs = curr_batch.tx_receipts.iter().enumerate().map(|(tx_id, tx)|
                   full_tx_json(curr_tx_num + tx_id, tx, &tx_id_to_event_range)
                ).collect::<Vec<_>>();

                test_helper(
                    vec![TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getBatches", [[random_batch_num], "Compact"]),
                        expected:
                        jsonrpc_result!([{"hash": format!("0x{batch_hash}"),"tx_range": {"start":first_tx_num,"end":last_tx_num},"custom_receipt": batch_receipt}])},
                    TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getBatches", [[random_batch_num], "Standard"]),
                        expected:
                        jsonrpc_result!([{"hash":format!("0x{batch_hash}"),"tx_range":{"start":first_tx_num,"end":last_tx_num},"txs":tx_hashes,"custom_receipt":batch_receipt}])},
                    TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getBatches", [[random_batch_num]]),
                        expected:
                        jsonrpc_result!([{"hash":format!("0x{batch_hash}"),"tx_range":{"start":first_tx_num,"end":last_tx_num},"txs":tx_hashes,"custom_receipt":batch_receipt}])},
                    TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getBatches", [random_batch_num]),
                        expected:
                        jsonrpc_result!([{"hash":format!("0x{batch_hash}"),"tx_range":{"start":first_tx_num,"end":last_tx_num},"txs":tx_hashes,"custom_receipt":batch_receipt}])},
                    TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getBatches", [[random_batch_num], "Full"]),
                        expected:
                        jsonrpc_result!([{"hash":format!("0x{batch_hash}"),"tx_range":{"start":first_tx_num,"end":last_tx_num},"txs":full_txs,"custom_receipt":batch_receipt}])},
                    ],
                    slots);

                return Ok(());
            }

            curr_batch_num += slot.batch_receipts().len();

            for batch in slot.batch_receipts(){
                curr_tx_num += batch.tx_receipts.len();
            }

        }

        let payload = jsonrpc_req!("ledger_getBatches", [[random_batch_num], "Compact"]);
        let expected = jsonrpc_result!([null]);
        test_helper(vec![TestExpect{payload, expected}], slots);
    }

    #[test]
    fn proptest_get_transactions((slots, tx_id_to_event_range, _total_num_batches) in arb_slots(10, 10), random_tx_num in 1..1000){
        let mut curr_tx_num = 1;

        let random_tx_num_usize = usize::try_from(random_tx_num).unwrap();

        for slot in &slots{
            for batch in slot.batch_receipts(){
                if curr_tx_num > random_tx_num_usize {
                    break;
                }

                if curr_tx_num + batch.tx_receipts.len() > random_tx_num_usize {
                    let tx_index = random_tx_num_usize - curr_tx_num;
                    let tx = batch.tx_receipts.get(tx_index).unwrap();

                    let tx_formatted = full_tx_json(curr_tx_num + tx_index, tx, &tx_id_to_event_range);

                    test_helper(vec![TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getTransactions", [[random_tx_num]]),
                        expected:
                        jsonrpc_result!([tx_formatted])},
                        TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getTransactions", [random_tx_num]),
                        expected:
                        jsonrpc_result!([tx_formatted])},
                        TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getTransactions", [[random_tx_num], "Compact"]),
                        expected:
                        jsonrpc_result!([tx_formatted])},
                        TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getTransactions", [[random_tx_num], "Standard"]),
                        expected:
                        jsonrpc_result!([tx_formatted])},
                        TestExpect{
                        payload:
                        jsonrpc_req!("ledger_getTransactions", [[random_tx_num], "Full"]),
                        expected:
                        jsonrpc_result!([tx_formatted])},
                        ]
                        , slots);

                    return Ok(());
                }

                curr_tx_num += batch.tx_receipts.len();
            }
        }

        let payload = jsonrpc_req!("ledger_getTransactions", [[random_tx_num]]);
        let expected = jsonrpc_result!([null]);
        test_helper(vec![TestExpect{payload, expected}], slots);

    }

    #[test]
    fn proptest_get_events((slots, tx_id_to_event_range, _total_num_batches) in arb_slots(10, 10), random_event_num in 1..10000){
        let mut curr_tx_num = 1;

        let random_event_num_usize = usize::try_from(random_event_num).unwrap();

        for slot in &slots {
            for batch in slot.batch_receipts(){
                for tx in &batch.tx_receipts{
                    let (start_event_range, end_event_range) = tx_id_to_event_range.get(&curr_tx_num).unwrap();
                    if *start_event_range > random_event_num_usize {
                        break;
                    }

                    if random_event_num_usize < *end_event_range {
                        let event_index = random_event_num_usize - *start_event_range;
                        let event: &Event = tx.events.get(event_index).unwrap();
                        let event_json = json!({
                            "key": event.key().inner(),
                            "value": event.value().inner(),
                        });

                        test_helper(vec![TestExpect{
                            payload:
                            jsonrpc_req!("ledger_getEvents", [random_event_num_usize]),
                            expected:
                            jsonrpc_result!([event_json])}]
                            , slots);

                        return Ok(());
                    }
                    curr_tx_num += 1;
                }
            }
        }

        let payload = jsonrpc_req!("ledger_getEvents", [random_event_num]);
        let expected = jsonrpc_result!([null]);
        test_helper(vec![TestExpect{payload, expected}], slots);
    }
);
