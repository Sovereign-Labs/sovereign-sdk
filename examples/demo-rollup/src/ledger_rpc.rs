use jsonrpsee::RpcModule;
use serde::{de::DeserializeOwned, Serialize};
use sov_rollup_interface::rpc::{
    BatchIdentifier, EventIdentifier, LedgerRpcProvider, QueryMode, SlotIdentifier, TxIdentifier,
};
use sovereign_db::ledger_db::LedgerDB;

/// Registers the following RPC methods
/// - `ledger_head`
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_head","params":[],"id":1}' http://127.0.0.1:12345`
/// - ledger_getSlots
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getSlots","params":[([SlotNumber(1)], QueryMode::Compact)],"id":1}' http://127.0.0.1:12345`
fn register_ledger_rpc_methods<B: Serialize + DeserializeOwned, T: Serialize + DeserializeOwned>(
    rpc: &mut RpcModule<LedgerDB>,
) -> Result<(), jsonrpsee::core::Error> {
    // #[derive(serde::Deserialize)]
    // struct IdsWithQueryMode<I>(Vec<I>, #[serde(default)] QueryMode)
    // where
    //     I: DeserializeOwned;
    rpc.register_method("ledger_getHead", move |_, db| {
        db.get_head::<B, T>().map_err(|e| e.into())
    })?;

    rpc.register_method("ledger_getSlots", move |params, db| {
        let ids: Vec<SlotIdentifier>;
        let query_mode: QueryMode;
        (ids, query_mode) = params.parse()?;
        db.get_slots::<B, T>(&ids, query_mode).map_err(|e| e.into())
    })?;

    rpc.register_method("ledger_getBatches", move |params, db| {
        let ids: Vec<BatchIdentifier>;
        let query_mode: QueryMode;
        (ids, query_mode) = params.parse()?;
        db.get_batches::<B, T>(&ids, query_mode)
            .map_err(|e| e.into())
    })?;

    rpc.register_method("ledger_getTransactions", move |params, db| {
        let ids: Vec<TxIdentifier>;
        let query_mode: QueryMode;
        (ids, query_mode) = params.parse()?;
        db.get_transactions::<T>(&ids, query_mode)
            .map_err(|e| e.into())
    })?;

    rpc.register_method("ledger_getEvents", move |params, db| {
        let ids: Vec<EventIdentifier> = params.parse()?;
        db.get_events(&ids).map_err(|e| e.into())
    })?;

    Ok(())
}

pub fn get_ledger_rpc<B: Serialize + DeserializeOwned, T: Serialize + DeserializeOwned>(
    ledger_db: LedgerDB,
) -> RpcModule<LedgerDB> {
    let mut rpc = RpcModule::new(ledger_db);
    register_ledger_rpc_methods::<B, T>(&mut rpc).expect("Failed to register ledger RPC methods");
    rpc
}
