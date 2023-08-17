use jsonrpsee::RpcModule;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::utils::to_jsonrpsee_error_object;
use sov_rollup_interface::rpc::{
    BatchIdentifier, EventIdentifier, LedgerRpcProvider, SlotIdentifier, TxIdentifier,
};

const LEDGER_RPC_ERROR: &str = "LEDGER_RPC_ERROR";

use self::query_args::{extract_query_args, QueryArgs};

/// Registers the following RPC methods
/// - `ledger_head`
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_head","params":[],"id":1}' http://127.0.0.1:12345`
/// - ledger_getSlots
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getSlots","params":[[1, 2], "Compact"],"id":1}' http://127.0.0.1:12345`
/// - ledger_getBatches
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[1, 2], "Standard"],"id":1}' http://127.0.0.1:12345`
/// - ledger_getTransactions
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getBatches","params":[[1, 2], "Full"],"id":1}' http://127.0.0.1:12345`
/// - ledger_getEvents
///    Example Query: `curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getBatches","params":[1, 2],"id":1}' http://127.0.0.1:12345`
fn register_ledger_rpc_methods<
    B: Serialize + DeserializeOwned + Clone + 'static,
    T: Serialize + DeserializeOwned + Clone + 'static,
>(
    rpc: &mut RpcModule<LedgerDB>,
) -> Result<(), jsonrpsee::core::Error> {
    rpc.register_method("ledger_getHead", move |_, db| {
        db.get_head::<B, T>()
            .map_err(|e| to_jsonrpsee_error_object(e, LEDGER_RPC_ERROR))
    })?;

    rpc.register_method("ledger_getSlots", move |params, db| {
        let args: QueryArgs<SlotIdentifier> = extract_query_args(params)?;
        db.get_slots::<B, T>(&args.0, args.1)
            .map_err(|e| to_jsonrpsee_error_object(e, LEDGER_RPC_ERROR))
    })?;

    rpc.register_method("ledger_getBatches", move |params, db| {
        let args: QueryArgs<BatchIdentifier> = extract_query_args(params)?;
        db.get_batches::<B, T>(&args.0, args.1)
            .map_err(|e| to_jsonrpsee_error_object(e, LEDGER_RPC_ERROR))
    })?;

    rpc.register_method("ledger_getTransactions", move |params, db| {
        let args: QueryArgs<TxIdentifier> = extract_query_args(params)?;
        db.get_transactions::<T>(&args.0, args.1)
            .map_err(|e| to_jsonrpsee_error_object(e, LEDGER_RPC_ERROR))
    })?;

    rpc.register_method("ledger_getEvents", move |params, db| {
        let ids: Vec<EventIdentifier> = params.parse()?;
        db.get_events(&ids)
            .map_err(|e| to_jsonrpsee_error_object(e, LEDGER_RPC_ERROR))
    })?;

    Ok(())
}

/// Register rpc methods for the provided `ledger_db`.
/// Calls the internal [`register_ledger_rpc_methods`] function.
pub fn get_ledger_rpc<
    B: Serialize + DeserializeOwned + Clone + 'static,
    T: Serialize + DeserializeOwned + Clone + 'static,
>(
    ledger_db: LedgerDB,
) -> RpcModule<LedgerDB> {
    let mut rpc = RpcModule::new(ledger_db);
    register_ledger_rpc_methods::<B, T>(&mut rpc).expect("Failed to register ledger RPC methods");
    rpc
}

mod query_args {
    use jsonrpsee::types::ErrorObjectOwned;
    use serde::de::DeserializeOwned;
    use sov_rollup_interface::rpc::QueryMode;

    /// A structure containing serialized query arguments for RPC queries.
    #[derive(serde::Deserialize)]
    pub struct QueryArgs<I>(pub Vec<I>, #[serde(default)] pub QueryMode);

    /// Extract the args from an RPC query, being liberal in what is accepted.
    /// To query for a list of items, users can either pass a list of ids, or tuple containing a list of ids and a query mode
    pub fn extract_query_args<I: DeserializeOwned>(
        params: jsonrpsee::types::Params,
    ) -> Result<QueryArgs<I>, ErrorObjectOwned> {
        if let Ok(args) = params.parse() {
            return Ok(args);
        }
        let ids: Vec<I> = params.parse()?;
        Ok(QueryArgs(ids, Default::default()))
    }
}
