use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use alloy_primitives::Address;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::pubsub::Params;
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
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;
use std::sync::Arc;
use std::time::Duration;

use crate::handlers::ETH_RPC_ERROR;

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
    let log_filter = match validate_params_for_log_subscription(parameters) {
        Ok(log_filter) => log_filter,
        Err(e) => {
            pending.reject(ErrorObjectOwned::from(e)).await;
            return Ok(());
        }
    };

    let accepted_sink = pending.accept().await?;

    let _task = tokio::spawn(async move {
        stream_logs(accepted_sink, log_filter, ethereum.clone()).await;
    });

    Ok(())
}

async fn stream_logs<S, Seq>(
    accepted_sink: SubscriptionSink,
    log_filter: Box<Filter>,
    ethereum: Arc<Ethereum<S, Seq>>,
) where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let evm = Evm::<S>::default();

    let mut x = 0;
    loop {
        let mut logs: Vec<alloy_rpc_types::Log> = Vec::new();

        let mut state = ethereum.sequencer.api_state().default_api_state_accessor();
        for i in 0..10 {
            let rec = evm.receipt(i, &mut state);
            match rec {
                Some(r) => {
                    for l in r.receipt.logs {
                        //println!("{:?}", l);
                        logs.push(l.into());
                    }
                }
                None => {}
            }
        }

        for l in logs {
            println!("log {:?}", l);

            let msg = SubscriptionMessage::new(
                accepted_sink.method_name(),
                accepted_sink.subscription_id(),
                &l,
            )
            .unwrap();

            //println!("msg {:?}", msg);
            accepted_sink.send(msg).await.unwrap();

            //println!("Send ==========");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn validate_params_for_log_subscription(
    parameters: JRpcParams<'static>,
) -> Result<Box<Filter>, ErrorObjectOwned> {
    let eth_subscribe = parameters.parse::<EthSubscribe>()?;

    let log_filter = match &eth_subscribe.kind {
        SubscriptionKind::Logs => match eth_subscribe.params {
            Params::Logs(filter) => filter,
            Params::Bool(_) => {
                return Err(to_jsonrpsee_error_object(
                    "Boolean parameters are not supported in LOG subscriptions.",
                    ETH_RPC_ERROR,
                ));
            }
            _ => Default::default(),
        },
        _ => {
            return Err(to_jsonrpsee_error_object(
                "Only LOG subscriptions are supported.",
                ETH_RPC_ERROR,
            ))
        }
    };

    Ok(log_filter)
}
