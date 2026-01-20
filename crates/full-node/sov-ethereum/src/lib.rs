mod handlers;

use std::convert::Infallible;

use alloy_primitives::{B256, U256};
use jsonrpsee::types::error::{
    CALL_EXECUTION_FAILED_CODE, INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE,
};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use sov_address::{EthereumAddress, FromVmAddress};
#[cfg(feature = "local")]
pub use sov_eth_dev_signer::Signers;
pub use sov_evm::EthereumAuthenticator;
use sov_evm::{convert_to_tx_signed, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::EthApiError;
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
    rpc.register_async_method("eth_gasPrice", |_, _, _| {
        // We don't use EVM gas price mechanism and rely on sov gas/gas price.
        // Therefore - we can safely return zero here as it's used by wallets to set gas price when sending transactions.
        // When we receive transactions - we override the gas price with 0 and disable charging the sender account for gas in handler.
        ready(Ok::<_, Infallible>(U256::ZERO))
    })?;
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

fn rpc_error_with_data(code: i32, message: &'static str, err: impl ToString) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(code, message, Some(err.to_string()))
}

pub(crate) fn rpc_invalid_params(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(INVALID_PARAMS_CODE, "Invalid params", err)
}

pub(crate) fn rpc_invalid_input(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(CALL_EXECUTION_FAILED_CODE, "Invalid input", err)
}

pub(crate) fn rpc_internal_error(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(INTERNAL_ERROR_CODE, "Internal error", err)
}

pub(crate) fn rpc_limit_exceeded(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(LIMIT_EXCEEDED_CODE, "Limit exceeded", err)
}

pub(crate) fn rpc_tx_rejected(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(TX_REJECTED_CODE, "Transaction rejected", err)
}

pub(crate) fn rpc_resource_not_found(err: impl ToString) -> ErrorObjectOwned {
    rpc_error_with_data(RESOURCE_NOT_FOUND_CODE, "Resource not found", err)
}
