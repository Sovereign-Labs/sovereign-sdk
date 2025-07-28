mod gas_price;
#[cfg(feature = "local")]
mod signer;

use std::sync::Arc;

use jsonrpsee::types::{ErrorCode, ErrorObjectOwned};
use jsonrpsee::RpcModule;
use reth_primitives::{Bytes, B256, U256};
pub use reth_rpc_eth_types::GasPriceOracleConfig;
use sov_address::{EthereumAddress, FromVmAddress};
#[cfg(feature = "local")]
pub use sov_eth_dev_signer::DevSigner;
pub use sov_evm::EthereumAuthenticator;
use sov_evm::{convert_to_transaction_signed, Evm, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{ApiStateAccessor, RawTx, Spec};
use sov_sequencer::Sequencer;

use crate::gas_price::gas_oracle::GasPriceOracle;

const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

#[derive(Clone)]
pub struct EthRpcConfig {
    pub gas_price_oracle_config: GasPriceOracleConfig,
    #[cfg(feature = "local")]
    pub eth_signer: DevSigner,
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
    rpc.register_async_method("eth_gasPrice", |_, ethereum, _| async move {
        let price = {
            let mut state = ethereum.api_state_accessor();

            let suggested_tip = ethereum
                .gas_price_oracle
                .suggest_tip_cap(&mut state)
                .await
                .unwrap();

            let evm = Evm::<S>::default();
            let base_fee = U256::from(
                evm.get_block_by_number(None, None, &mut state)
                    .unwrap()
                    .unwrap()
                    .header
                    .base_fee_per_gas
                    .unwrap_or_default(),
            );

            suggested_tip + base_fee
        };

        Ok::<U256, ErrorObjectOwned>(price)
    })?;

    rpc.register_async_method(
        "eth_sendRawTransaction",
        |parameters, ethereum, _| async move {
            let data: Bytes = parameters.one().unwrap();

            let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };

            let (tx_hash, raw_message) = ethereum
                .make_raw_tx(raw_evm_tx)
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));

            ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
                to_jsonrpsee_error_object(
                    format!("{} - '{}' ({:?})", e.status, e.message, e.details),
                    ETH_RPC_ERROR,
                )
            })?;

            Ok::<_, ErrorObjectOwned>(tx_hash)
        },
    )?;

    #[cfg(feature = "local")]
    signer::register_signer_rpc_methods::<S, Seq>(rpc)?;

    Ok(())
}

struct Ethereum<S: Spec, Seq: Sequencer<Spec = S>> {
    sequencer: Arc<Seq>,
    gas_price_oracle: GasPriceOracle<S>,
    #[cfg(feature = "local")]
    eth_signer: DevSigner,
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

        Ok((tx_hash, message))
    }
}

pub(crate) fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
