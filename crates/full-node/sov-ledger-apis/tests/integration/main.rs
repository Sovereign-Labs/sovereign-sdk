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
async fn get_batch() {
    let batch = ledger_response_body(|client| async move {
        client
            .get_batch_by_id(&IntOrHash::Integer(0), None)
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
                .get_batch_by_slot_id_and_offset(&IntOrHash::Integer(0), 0, None)
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
            .get_tx_by_id(&IntOrHash::Integer(0), None)
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
                .get_tx_by_id(&IntOrHash::Integer(0), None)
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
                .get_tx_by_slot_id_and_offset(&IntOrHash::Integer(0), 0, 0, None)
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
                .get_tx_by_batch_id_and_offset(&IntOrHash::Integer(0), 0, None)
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

    let response = client.get_event_by_id(0).await.unwrap();

    assert_eq!(response.status(), 200);
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!(*response);
    });
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
