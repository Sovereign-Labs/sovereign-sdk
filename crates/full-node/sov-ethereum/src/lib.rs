mod gas_price;
mod handlers;

use std::sync::Arc;

use alloy_primitives::B256;
use jsonrpsee::types::{ErrorCode, ErrorObjectOwned};
use jsonrpsee::RpcModule;
pub use reth_rpc_eth_types::GasPriceOracleConfig;
use sov_address::{EthereumAddress, FromVmAddress};
#[cfg(feature = "local")]
pub use sov_eth_dev_signer::Signers;
pub use sov_evm::EthereumAuthenticator;
use sov_evm::{convert_to_transaction_signed, Evm, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_sequencer::Sequencer;

use crate::gas_price::gas_oracle::GasPriceOracle;

#[derive(Clone)]
pub struct EthRpcConfig {
    pub gas_price_oracle_config: GasPriceOracleConfig,
    #[cfg(feature = "local")]
    pub eth_signer: Signers,
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
        gas_price_oracle_config,
    } = eth_rpc_config;

    let mut rpc = RpcModule::new(Ethereum {
        sequencer,
        gas_price_oracle: GasPriceOracle::new(Evm::<S>::default(), gas_price_oracle_config),
        #[cfg(feature = "local")]
        eth_signer,
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
    rpc.register_async_method("eth_gasPrice", handlers::eth_gas_price)?;
    rpc.register_async_method("eth_sendRawTransaction", handlers::eth_send_raw_transaction)?;

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
    gas_price_oracle: GasPriceOracle<S>,
    #[cfg(feature = "local")]
    eth_signer: Signers,
}

impl<S, Seq> Ethereum<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    fn api_state_accessor(&self) -> ApiStateAccessor<S> {
        self.sequencer.api_state().default_api_state_accessor()
    }

    fn make_raw_tx(&self, raw_tx: RlpEvmTransaction) -> Result<(B256, Vec<u8>), ErrorObjectOwned> {
        let signed_transaction = convert_to_transaction_signed(raw_tx.clone())
            // TODO: Fix this later
            .map_err(|_err| ErrorCode::ServerError(500))?;

        let tx_hash = signed_transaction.hash();
        let message = borsh::to_vec(&raw_tx).expect("Failed to serialize raw tx");

        Ok((*tx_hash, message))
    }
}

pub(crate) fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
