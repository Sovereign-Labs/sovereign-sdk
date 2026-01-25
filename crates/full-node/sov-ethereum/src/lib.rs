mod handlers;

use std::convert::Infallible;

use alloy_primitives::{B256, U256};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use sov_address::{EthereumAddress, FromVmAddress};
#[cfg(feature = "local")]
pub use sov_eth_dev_signer::Signers;
pub use sov_evm::EthereumAuthenticator;
use sov_evm::{convert_to_tx_signed, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::{
    internal_rpc_err, invalid_params_rpc_err, rpc_error_with_code, EthApiError,
};
use sov_sequencer::{SeqConfigExtension, Sequencer};
use std::future::ready;

pub use handlers::Cursor;

use crate::handlers::Handlers;

#[derive(Clone)]
pub struct EthRpcConfig {
    #[cfg(feature = "local")]
    pub eth_signer: Signers,
    pub extension: SeqConfigExtension,
    /// Shutdown signal receiver for graceful termination
    pub shutdown_receiver: tokio::sync::watch::Receiver<()>,
}

const LIMIT_EXCEEDED_CODE: i32 = -32005;
const METHOD_NOT_SUPPORTED_CODE: i32 = -32004;
const RESOURCE_NOT_FOUND_CODE: i32 = -32001;
const TX_REJECTED_CODE: i32 = -32003;

pub fn get_ethereum_rpc<S, Seq>(eth_rpc_config: EthRpcConfig, sequencer: Seq) -> RpcModule<()>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    // Unpack config
    let EthRpcConfig {
        #[cfg(feature = "local")]
        eth_signer,
        extension,
        shutdown_receiver,
    } = eth_rpc_config;

    let mut rpc = RpcModule::new(Ethereum {
        sequencer,
        #[cfg(feature = "local")]
        eth_signer,
        extension,
        shutdown_receiver,
    });

    register_rpc_methods::<S, Seq>(&mut rpc).expect("Failed to register sequencer RPC methods");

    rpc.remove_context()
}

fn register_rpc_methods<S, Seq>(
    rpc: &mut RpcModule<Ethereum<S, Seq>>,
) -> Result<(), jsonrpsee::core::client::Error>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    for method in [
        "eth_protocolVersion",
        "eth_coinbase",
        "eth_mining",
        "eth_hashrate",
        "eth_getTransactionByBlockHashAndIndex",
        "eth_getTransactionByBlockNumberAndIndex",
        "eth_getUncleCountByBlockHash",
        "eth_getUncleCountByBlockNumber",
        "eth_getUncleByBlockHashAndIndex",
        "eth_getUncleByBlockNumberAndIndex",
        "eth_newFilter",
        "eth_newBlockFilter",
        "eth_newPendingTransactionFilter",
        "eth_uninstallFilter",
        "eth_getFilterChanges",
        "eth_getFilterLogs",
        "eth_sign",
        "eth_signTransaction",
        "eth_signTypedData",
        "eth_signTypedData_v1",
        "eth_signTypedData_v3",
        "eth_signTypedData_v4",
        "eth_getProof",
        "eth_createAccessList",
        "eth_syncing",
        "net_peerCount",
        "trace_block",
        "trace_call",
        "trace_filter",
        "trace_get",
        "trace_rawTransaction",
        "trace_replayBlockTransactions",
        "trace_replayTransaction",
        "trace_transaction",
        "txpool_content",
        "txpool_contentFrom",
        "txpool_inspect",
        "txpool_status",
    ] {
        let method_name = method;
        rpc.register_async_method(method_name, move |_, _, _| {
            ready(Err::<(), _>(rpc_method_not_supported(method_name)))
        })?;
    }

    rpc.register_async_method("eth_sendRawTransaction", Handlers::eth_send_raw_transaction)?;
    rpc.register_async_method(
        "eth_sendRawTransactionSync",
        Handlers::eth_send_raw_transaction_sync,
    )?;
    rpc.register_async_method(
        "realtime_sendRawTransaction",
        Handlers::realtime_send_raw_transaction,
    )?;

    rpc.register_async_method("eth_getLogs", handlers::LogHandlers::<S, Seq>::eth_get_logs)?;
    rpc.register_async_method(
        "eth_getLogsWithCursor",
        handlers::LogHandlers::<S, Seq>::eth_get_logs_with_cursor,
    )?;
    rpc.register_subscription(
        "eth_subscribe",
        "eth_subscription",
        "eth_unsubscribe",
        handlers::eth_subscribe,
    )?;

    #[cfg(feature = "local")]
    {
        rpc.register_async_method("eth_accounts", Handlers::eth_accounts)?;
        rpc.register_async_method("eth_sendTransaction", Handlers::eth_send_transaction)?;
    }

    Ok(())
}

struct Ethereum<S: Spec, Seq: Sequencer<Spec = S>> {
    sequencer: Seq,
    #[cfg(feature = "local")]
    eth_signer: Signers,
    extension: SeqConfigExtension,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
}

impl<S, Seq> Ethereum<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    fn make_raw_tx(&self, raw_tx: RlpEvmTransaction) -> Result<(B256, Vec<u8>), ErrorObjectOwned> {
        let message = borsh::to_vec(&raw_tx).expect("Failed to serialize raw tx");
        let signed_transaction = convert_to_tx_signed(raw_tx)
            .map_err(|err| ErrorObjectOwned::from(EthApiError::from(err)))?;

        let tx_hash = signed_transaction.hash();

        Ok((*tx_hash, message))
    }

    fn api_state_accessor(&self) -> ApiStateAccessor<S> {
        self.sequencer
            .api_state()
            .build_api_state_accessor(None)
            .expect("Failed to build api state accessor")
    }
}

pub(crate) fn rpc_invalid_params(err: impl ToString) -> ErrorObjectOwned {
    invalid_params_rpc_err(err.to_string())
}

pub(crate) fn rpc_internal_error(err: impl ToString) -> ErrorObjectOwned {
    internal_rpc_err(err.to_string())
}

pub(crate) fn rpc_limit_exceeded(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_code(LIMIT_EXCEEDED_CODE, err.to_string())
}

pub(crate) fn rpc_method_not_supported(method: &str) -> ErrorObjectOwned {
    rpc_error_with_code(
        METHOD_NOT_SUPPORTED_CODE,
        format!("Method {method} not supported"),
    )
}

pub(crate) fn rpc_tx_rejected(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_code(TX_REJECTED_CODE, err.to_string())
}

pub(crate) fn rpc_resource_not_found(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_code(RESOURCE_NOT_FOUND_CODE, err.to_string())
}
