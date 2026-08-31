//! Basic request-response tests based on the ledger data supplied by
//! [`sov_test_utils::ledger_db::materialize_simple_ledger_db_data`].

use std::future::Future;
use std::str::FromStr;

use assert_json_diff::assert_json_eq;
use sov_api_spec::types;
use sov_api_spec::types::IntOrHash;
use sov_test_utils::ledger_db::{LedgerTestService, LedgerTestServiceData};
use utils::ledger_response_body;

#[tokio::test(flavor = "multi_thread")]
async fn get_latest_slot() {
    let slot = ledger_response_body(|client| async move {
        client.get_latest_slot(None).await.unwrap().into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&slot);
    });

    // By number.
    let rollup_height = slot["number"].as_u64().unwrap();
    assert_json_eq!(
        slot,
        ledger_response_body(move |client| async move {
            client
                .get_slot_by_id(&IntOrHash::Integer(rollup_height), None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_latest_slot_include_children() {
    let slot = ledger_response_body(|client| async move {
        client
            .get_latest_slot(Some(types::GetLatestSlotChildren::X1))
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&slot);
    });

    // By number.
    let rollup_height = slot["number"].as_u64().unwrap();
    assert_json_eq!(
        slot,
        ledger_response_body(move |client| async move {
            client
                .get_slot_by_id(
                    &IntOrHash::Integer(rollup_height),
                    Some(types::GetSlotByIdChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_finalized_slot() {
    let slot = ledger_response_body(|client| async move {
        client.get_finalized_slot(None).await.unwrap().into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&slot);
    });

    // By number.
    let rollup_height = slot["number"].as_u64().unwrap();
    assert_json_eq!(
        slot,
        ledger_response_body(move |client| async move {
            client
                .get_slot_by_id(&IntOrHash::Integer(rollup_height), None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_finalized_slot_include_children() {
    let slot = ledger_response_body(|client| async move {
        client
            .get_finalized_slot(Some(types::GetFinalizedSlotChildren::X1))
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&slot);
    });

    // By number.
    let rollup_height = slot["number"].as_u64().unwrap();
    assert_json_eq!(
        slot,
        ledger_response_body(move |client| async move {
            client
                .get_slot_by_id(
                    &IntOrHash::Integer(rollup_height),
                    Some(types::GetSlotByIdChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_batch() {
    let batch = ledger_response_body(|client| async move {
        client
            .get_batch_by_id(&IntOrHash::Integer(3), None)
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&batch);
    });

    // By hash.
    let hash = types::Hash::from_str(batch["hash"].as_str().unwrap()).unwrap();
    assert_json_eq!(
        batch,
        ledger_response_body(|client| async move {
            client
                .get_batch_by_id(&IntOrHash::Hash(hash), None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By slot offset.
    assert_json_eq!(
        batch,
        ledger_response_body(|client| async move {
            client
                .get_batch_by_slot_id_and_offset(&IntOrHash::Integer(1), 1, None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_batch_include_children() {
    let batch = ledger_response_body(|client| async move {
        client
            .get_batch_by_id(
                &IntOrHash::Integer(3),
                Some(types::GetBatchByIdChildren::X1),
            )
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&batch);
    });

    // By hash.
    let hash = types::Hash::from_str(batch["hash"].as_str().unwrap()).unwrap();
    assert_json_eq!(
        batch,
        ledger_response_body(|client| async move {
            client
                .get_batch_by_id(
                    &IntOrHash::Hash(hash),
                    Some(types::GetBatchByIdChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By slot offset.
    assert_json_eq!(
        batch,
        ledger_response_body(|client| async move {
            client
                .get_batch_by_slot_id_and_offset(
                    &IntOrHash::Integer(1),
                    1,
                    Some(types::GetBatchBySlotIdAndOffsetChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

/// All ways of getting a transaction should return the same transaction data.
#[tokio::test(flavor = "multi_thread")]
async fn get_tx() {
    let tx = ledger_response_body(|client| async move {
        client
            .get_tx_by_id(&IntOrHash::Integer(7), None)
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&tx);
    });

    // By number.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_id(&IntOrHash::Integer(7), None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By hash.
    let hash = types::Hash::from_str(tx["hash"].as_str().unwrap()).unwrap();
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_id(&IntOrHash::Hash(hash), None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By slot offset.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_slot_id_and_offset(&IntOrHash::Integer(1), 1, 1, None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By batch offset.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_batch_id_and_offset(&IntOrHash::Integer(3), 1, None)
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_tx_include_children() {
    let tx = ledger_response_body(|client| async move {
        client
            .get_tx_by_id(&IntOrHash::Integer(7), Some(types::GetTxByIdChildren::X1))
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    // Check for regressions in the response format.
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(&tx);
    });

    // By number.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_id(&IntOrHash::Integer(7), Some(types::GetTxByIdChildren::X1))
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By hash.
    let hash = types::Hash::from_str(tx["hash"].as_str().unwrap()).unwrap();
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_id(&IntOrHash::Hash(hash), Some(types::GetTxByIdChildren::X1))
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By slot offset.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_slot_id_and_offset(
                    &IntOrHash::Integer(1),
                    1,
                    1,
                    Some(types::GetTxBySlotIdAndOffsetChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );

    // By batch offset.
    assert_json_eq!(
        tx,
        ledger_response_body(|client| async move {
            client
                .get_tx_by_batch_id_and_offset(
                    &IntOrHash::Integer(3),
                    1,
                    Some(types::GetTxByBatchIdAndOffsetChildren::X1),
                )
                .await
                .unwrap()
                .into_inner()
        })
        .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_event() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Simple)
        .await
        .unwrap();
    let client = ledger_service.axum_client;

    let response = client.get_event_by_id(1).await.unwrap();

    assert_eq!(response.status(), 200);
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(*response);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_respects_page_size() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Simple)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let events: serde_json::Value = reqwest::get(format!(
        "http://{addr}/ledger/events?page=first&page[size]=1"
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();

    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["number"].as_u64(), Some(0));
    assert_eq!(events[0]["key"].as_str(), Some("foo0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_prefix_reads_past_first_raw_page() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Complex)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let events: serde_json::Value =
        reqwest::get(format!("http://{addr}/ledger/events?prefix=foo100"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["number"].as_u64(), Some(200));
    assert_eq!(events[0]["key"].as_str(), Some("foo100"));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_prefix_rejects_deep_cursor() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Complex)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let response = reqwest::get(format!(
        "http://{addr}/ledger/events?prefix=foo&page=next&page[cursor]=10001&page[size]=10"
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_prefix_paginates() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::Complex)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    // Complex fixture emits foo{N} at event number 2N for N in 0..266.
    let page1: serde_json::Value = reqwest::get(format!(
        "http://{addr}/ledger/events?prefix=foo&page=first&page[size]=5"
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let page1 = page1.as_array().unwrap();
    let page1_numbers = page1
        .iter()
        .map(|e| e["number"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(page1_numbers, vec![0, 2, 4, 6, 8]);

    let next_cursor = page1_numbers.last().unwrap() + 1;
    let page2: serde_json::Value = reqwest::get(format!(
        "http://{addr}/ledger/events?prefix=foo&page=next&page[cursor]={next_cursor}&page[size]=5"
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let page2 = page2.as_array().unwrap();
    let page2_numbers = page2
        .iter()
        .map(|e| e["number"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(page2_numbers, vec![10, 12, 14, 16, 18]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_latest_aggregated_proof() {
    let response = ledger_response_body(|client| async move {
        client
            .get_latest_aggregated_proof()
            .await
            .unwrap()
            .into_inner()
    })
    .await;

    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(response);
    });
}

mod utils {
    use super::*;

    pub async fn ledger_response_body<T, F, Fut>(api_call: F) -> serde_json::Value
    where
        F: FnOnce(sov_api_spec::Client) -> Fut + Send + Sync + 'static,
        T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
        Fut: Future<Output = T> + Send + Sync + 'static,
    {
        let ledger_service = LedgerTestService::new(LedgerTestServiceData::Complex)
            .await
            .unwrap();
        let client = ledger_service.axum_client;

        let response_data = api_call(client).await;
        serde_json::to_value(&response_data)
            .expect("Failed test; the API response can't be serialized as JSON... this is a bug")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_by_key_paginates_by_event_number() {
    use sov_test_utils::ledger_db::{REPEATED_EVENT_COUNT, REPEATED_EVENT_KEY};

    let ledger_service = LedgerTestService::new(LedgerTestServiceData::RepeatedEventKey)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let mut seen = Vec::new();
    let mut url = format!(
        "http://{addr}/ledger/events/by-key?key={REPEATED_EVENT_KEY}&page=first&page[size]=10"
    );
    loop {
        let page: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
        for event in page["events"].as_array().unwrap() {
            assert_eq!(event["key"].as_str(), Some(REPEATED_EVENT_KEY));
            seen.push(event["number"].as_u64().unwrap());
        }
        match page["next"].as_str() {
            Some(cursor) => {
                url = format!(
                    "http://{addr}/ledger/events/by-key?key={REPEATED_EVENT_KEY}\
                     &page=next&page[cursor]={cursor}&page[size]=10"
                );
            }
            None => break,
        }
    }

    // Every event exactly once, in order, with no duplicates across pages.
    assert_eq!(seen.len() as u64, REPEATED_EVENT_COUNT);
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted, seen);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_by_key_rejects_empty_key_and_bad_cursor() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::RepeatedEventKey)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let empty = reqwest::get(format!("http://{addr}/ledger/events/by-key?key="))
        .await
        .unwrap();
    assert_eq!(empty.status(), 400);

    let bad_cursor = reqwest::get(format!(
        "http://{addr}/ledger/events/by-key?key=Repeated/Event&page=next&page[cursor]=abc"
    ))
    .await
    .unwrap();
    assert_eq!(bad_cursor.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_events_by_key_unknown_key_is_empty() {
    let ledger_service = LedgerTestService::new(LedgerTestServiceData::RepeatedEventKey)
        .await
        .unwrap();
    let addr = ledger_service.axum_handle.listening().await.unwrap();

    let page: serde_json::Value =
        reqwest::get(format!("http://{addr}/ledger/events/by-key?key=Nope/Nope"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert!(page["events"].as_array().unwrap().is_empty());
    assert!(page["next"].is_null());
}
