#![allow(dead_code)]
use alloy_primitives::Address;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::{Filter, Log};
use jsonrpsee::core::SubscriptionError;
use jsonrpsee::types::{ErrorCode, ErrorObjectOwned, Params as JRpcParams};
use jsonrpsee::PendingSubscriptionSink;
use jsonrpsee::SubscriptionMessage;
use jsonrpsee::{Extensions, SubscriptionSink};
use reth_primitives::LogData;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
#[cfg(feature = "local")]
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;
use std::sync::Arc;
use std::time::Duration;

use crate::to_jsonrpsee_error_object;
use crate::Ethereum;

const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

#[cfg(feature = "local")]
pub(crate) mod signer {
    use super::*;
    use alloy_eips::Encodable2718;
    use alloy_primitives::Address;
    use alloy_rpc_types::TransactionRequest;
    use reth_rpc_eth_types::EthApiError;
    use sov_evm::eth_api_into_rpc_error;
    use sov_modules_api::macros::config_value;

    pub async fn eth_accounts<S, Seq>(
        _: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<Vec<Address>, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        Ok(ethereum.eth_signer.addresses())
    }

    pub async fn eth_send_transaction<S, Seq>(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<B256, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        let mut transaction_request: TransactionRequest = parameters.one()?;

        let evm = Evm::<S>::default();

        // get from, return error if none
        let from = transaction_request
            .from
            .ok_or(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR))?;

        // return error if not in signers
        if !ethereum.eth_signer.addresses().contains(&from) {
            return Err(to_jsonrpsee_error_object(
                "From address not in signers",
                ETH_RPC_ERROR,
            ));
        }

        let raw_evm_tx = {
            let mut state = ethereum.sequencer.api_state().default_api_state_accessor();

            // set nonce if none
            transaction_request.nonce.get_or_insert_with(|| {
                evm.get_transaction_count(from, None, &mut state)
                    .unwrap_or_default()
                    .to::<u64>()
            });

            let chain_id = evm
                .chain_id(&mut state)
                .expect("Failed to get chain id")
                .map(|id| id.to())
                .unwrap_or(config_value!("CHAIN_ID"));
            transaction_request.chain_id = Some(chain_id);

            let estimated_gas = evm.eth_estimate_gas(
                transaction_request.clone(),
                Some("pending".to_string()),
                &mut state,
            )?;
            transaction_request.gas = Some(estimated_gas.to::<u64>());

            let transaction = transaction_request
                .build_typed_tx()
                .map_err(|_| eth_api_into_rpc_error(EthApiError::TransactionConversionError))?;

            // sign transaction
            let signed_tx = ethereum
                .eth_signer
                .sign_transaction(transaction, &from)
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            RlpEvmTransaction {
                rlp: signed_tx.encoded_2718(),
            }
        };
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

        Ok(tx_hash)
    }
}

pub async fn eth_send_raw_transaction<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<B256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let data: Bytes = parameters.one()?;

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

    Ok(tx_hash)
}

use alloy_rpc_types_eth::pubsub::Params;

#[derive(Debug, Clone, serde::Deserialize)]
struct EthSubscribe {
    kind: SubscriptionKind,
    params: Params,
}

pub async fn eth_subscribe<S, Seq>(
    parameters: JRpcParams<'static>,
    pending: PendingSubscriptionSink,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> jsonrpsee::core::SubscriptionResult
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let eth_subscribe = match parameters.parse::<EthSubscribe>() {
        Ok(eth_subscribe) => eth_subscribe,
        Err(e) => {
            pending.reject(ErrorObjectOwned::from(e)).await;
            return Ok(());
        }
    };

    let accepted_sink = pending.accept().await?;

    let log_filter = match &eth_subscribe.kind {
        SubscriptionKind::Logs => match eth_subscribe.params {
            Params::Logs(filter) => filter,
            Params::Bool(_) => {
                return Err(to_jsonrpsee_error_object(
                    "Boolean parameters are not supported in LOG subscriptions.",
                    ETH_RPC_ERROR,
                )
                .into());
            }
            _ => Default::default(),
        },
        _ => {
            return Err(to_jsonrpsee_error_object(
                "Only LOG subscriptions are supported.",
                ETH_RPC_ERROR,
            )
            .into())
        }
    };

    let _task = tokio::spawn(async move {
        stream_logs(accepted_sink, log_filter, ethereum.clone()).await;
    });

    Ok(())
}

async fn stream_logs<S, Seq>(
    accepted_sink: SubscriptionSink,
    log_filter: Box<Filter>,
    _ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let mut x = 0;

    tokio::time::sleep(Duration::from_millis(1)).await;

    let log = Log {
        inner: alloy_primitives::Log {
            address: Address::parse_checksummed("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", None)
                .unwrap(),

            data: LogData::new_unchecked(vec![], Default::default()),
        },
        block_hash: None,
        block_number: None,
        block_timestamp: None,
        transaction_hash: None,
        transaction_index: None,
        log_index: None,
        removed: false,
    };

    let msg = SubscriptionMessage::new(
        accepted_sink.method_name(),
        accepted_sink.subscription_id(),
        &log,
    )
    .unwrap();

    accepted_sink.send(msg).await.unwrap();
}
