mod handlers;

use std::convert::Infallible;
use std::sync::Arc;

use alloy_primitives::{B256, U256};
use jsonrpsee::types::{ErrorCode, ErrorObjectOwned};
use jsonrpsee::RpcModule;
use sov_address::{EthereumAddress, FromVmAddress};
#[cfg(feature = "local")]
pub use sov_eth_dev_signer::Signers;
pub use sov_evm::EthereumAuthenticator;
use sov_evm::{convert_to_transaction_signed, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_sequencer::{SeqConfigExtension, Sequencer};
use std::future::ready;

#[derive(Clone)]
pub struct EthRpcConfig {
    #[cfg(feature = "local")]
    pub eth_signer: Signers,
    pub extension: SeqConfigExtension,
}

pub fn get_ethereum_rpc<S, Seq>(eth_rpc_config: EthRpcConfig, sequencer: Arc<Seq>) -> RpcModule<()>
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
    } = eth_rpc_config;

    let mut rpc = RpcModule::new(Ethereum {
        sequencer,
        #[cfg(feature = "local")]
        eth_signer,
        extension,
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
    rpc.register_async_method("eth_sendRawTransaction", handlers::eth_send_raw_transaction)?;
    rpc.register_async_method(
        "realtime_sendRawTransaction",
        handlers::realtime_send_raw_transaction,
    )?;

    rpc.register_async_method("eth_getLogs", handlers::eth_get_logs)?;
    rpc.register_subscription(
        "eth_subscribe",
        "eth_subscription",
        "eth_unsubscribe",
        handlers::eth_subscribe,
    )?;

    #[cfg(feature = "local")]
    {
        rpc.register_async_method("eth_accounts", handlers::signer::eth_accounts)?;
        rpc.register_async_method(
            "eth_sendTransaction",
            handlers::signer::eth_send_transaction,
        )?;
    }

    Ok(())
}

struct Ethereum<S: Spec, Seq: Sequencer<Spec = S>> {
    sequencer: Arc<Seq>,
    #[cfg(feature = "local")]
    eth_signer: Signers,
    extension: SeqConfigExtension,
}

impl<S, Seq> Ethereum<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    fn make_raw_tx(&self, raw_tx: RlpEvmTransaction) -> Result<(B256, Vec<u8>), ErrorObjectOwned> {
        let signed_transaction = convert_to_transaction_signed(raw_tx.clone())
            // TODO: Fix this later
            .map_err(|_err| ErrorCode::ServerError(500))?;

        let tx_hash = signed_transaction.hash();
        let message = borsh::to_vec(&raw_tx).expect("Failed to serialize raw tx");

        Ok((*tx_hash, message))
    }

    fn api_state_accessor(&self) -> ApiStateAccessor<S> {
        self.sequencer
            .api_state()
            .build_api_state_accessor(None)
            .expect("Failed to build api state accessor")
    }
}

pub(crate) fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
