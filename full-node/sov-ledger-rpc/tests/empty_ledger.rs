use std::net::SocketAddr;
use std::sync::Arc;

use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::params::ArrayParams;
use sov_db::ledger_db::LedgerDB;
use sov_ledger_rpc::client::RpcClient;
use sov_ledger_rpc::server::rpc_module;
use sov_ledger_rpc::HexHash;
use sov_modules_api::Event;
use sov_rollup_interface::rpc::{
    BatchResponse, EventIdentifier, QueryMode, SlotResponse, TxIdAndOffset, TxIdentifier,
    TxResponse,
};
use tempfile::tempdir;

async fn rpc_server() -> (jsonrpsee::server::ServerHandle, SocketAddr) {
    let dir = tempdir().unwrap();
    let db = LedgerDB::with_path(dir).unwrap();
    let rpc_module = rpc_module::<LedgerDB, u32, u32>(db).unwrap();

    let server = jsonrpsee::server::ServerBuilder::default()
        .build("127.0.0.1:0")
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    (server.start(rpc_module), addr)
}

async fn rpc_client(
    addr: SocketAddr,
) -> Arc<
    impl RpcClient<SlotResponse<u32, u32>, BatchResponse<u32, u32>, TxResponse<u32>>
        + SubscriptionClientT,
> {
    Arc::new(
        jsonrpsee::ws_client::WsClientBuilder::new()
            .build(format!("ws://{}", addr))
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn getters_succeed() {
    let (_server_handle, addr) = rpc_server().await;
    let rpc_client = rpc_client(addr).await;

    rpc_client.get_head(QueryMode::Compact).await.unwrap();
    rpc_client.get_head(QueryMode::Standard).await.unwrap();
    rpc_client.get_head(QueryMode::Full).await.unwrap();

    rpc_client
        .get_slots(vec![], QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_batches(vec![], QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_transactions(vec![], QueryMode::Compact)
        .await
        .unwrap();
    rpc_client.get_events(vec![]).await.unwrap();

    let hash = HexHash([0; 32]);
    rpc_client
        .get_slot_by_hash(hash, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_batch_by_hash(hash, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_tx_by_hash(hash, QueryMode::Compact)
        .await
        .unwrap();

    rpc_client
        .get_slot_by_number(0, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_batch_by_number(0, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_tx_by_number(0, QueryMode::Compact)
        .await
        .unwrap();

    rpc_client
        .get_slots_range(0, 1, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_batches_range(0, 1, QueryMode::Compact)
        .await
        .unwrap();
    rpc_client
        .get_txs_range(0, 1, QueryMode::Compact)
        .await
        .unwrap();
}

#[tokio::test]
async fn subscribe_slots_succeeds() {
    let (_server_handle, addr) = rpc_server().await;
    let rpc_client = rpc_client(addr).await;

    rpc_client.subscribe_slots().await.unwrap();
}

#[tokio::test]
async fn get_head_with_optional_query_mode() {
    let (_server_handle, addr) = rpc_server().await;
    let rpc_client = rpc_client(addr).await;

    // No QueryMode param.
    {
        rpc_client
            .request::<serde_json::Value, _>("ledger_getHead", ArrayParams::new())
            .await
            .unwrap();
    }
    // With QueryMode param.
    {
        let mut params = ArrayParams::new();
        params.insert(QueryMode::Standard).unwrap();
        rpc_client
            .request::<serde_json::Value, _>("ledger_getHead", params)
            .await
            .unwrap();
    }
}

/// `ledger_getEvents` supports several parameter types, because of a
/// `jsonrpsee` limitation. See:
/// - https://github.com/Sovereign-Labs/sovereign-sdk/pull/1058
/// - https://github.com/Sovereign-Labs/sovereign-sdk/issues/1037
///
/// While `jsonrpsee` macro-generated clients can only generate nested array
/// types as parameters (e.g. `"params": [[1, 2, 3]]`), we want to test that
/// non-nested array types are also supported (e.g. `"params": [1, 2, 3]` and
/// `"params": [{"txId": 1, "offset": 2}]`).
#[tokio::test]
async fn get_events_patterns() {
    let (_server_handle, addr) = rpc_server().await;
    let rpc_client = rpc_client(addr).await;

    rpc_client
        .get_events(vec![EventIdentifier::Number(2)])
        .await
        .unwrap();
    rpc_client
        .request::<Vec<Option<Event>>, _>("ledger_getEvents", vec![vec![2]])
        .await
        .unwrap();
    rpc_client
        .request::<Vec<Option<Event>>, _>("ledger_getEvents", vec![2])
        .await
        .unwrap();
    rpc_client
        .request::<Vec<Option<Event>>, _>(
            "ledger_getEvents",
            vec![EventIdentifier::TxIdAndOffset(TxIdAndOffset {
                tx_id: TxIdentifier::Number(1),
                offset: 2,
            })],
        )
        .await
        .unwrap();
}
