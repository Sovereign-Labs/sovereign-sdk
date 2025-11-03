use crate::handlers::subscribe::SubscriptionRequest;
use crate::handlers::ETH_RPC_ERROR;
use crate::to_jsonrpsee_error_object;
use alloy_rpc_types::pubsub::Params;
use alloy_rpc_types::pubsub::SubscriptionKind;
use alloy_rpc_types::FilterBlockOption;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Block Option parameters are not supported in LOG subscriptions. Please use eth_getLogs or eth_getLogsWithCursor")]
    BlockOptionParam,
    #[error("Boolean parameters are not supported in LOG subscriptions")]
    BoolParam,
    #[error("Only LOG and newHeads subscriptions are supported")]
    OnlyLogAndNewHeadsSubscription,
    #[error("newHeads subscription does not accept parameters")]
    NewHeadsDoesNotAcceptParams,
}

impl From<Error> for ErrorObjectOwned {
    fn from(err: Error) -> Self {
        to_jsonrpsee_error_object(err, ETH_RPC_ERROR)
    }
}

pub fn parse(
    parameters: JRpcParams<'static>,
) -> Result<(SubscriptionKind, Params), ErrorObjectOwned> {
    let mut parameters = parameters.sequence();
    let kind = parameters.next()?;
    let params = parameters.optional_next()?.unwrap_or_default();
    Ok((kind, params))
}

pub fn validate(kind: SubscriptionKind, params: Params) -> Result<SubscriptionRequest, Error> {
    match kind {
        SubscriptionKind::NewHeads => {
            if params != Params::None {
                return Err(Error::NewHeadsDoesNotAcceptParams);
            }
            Ok(SubscriptionRequest::Heads)
        }
        SubscriptionKind::Logs => {
            let filter = match params {
                Params::Logs(filter) => {
                    if filter.block_option == FilterBlockOption::default() {
                        filter
                    } else {
                        return Err(Error::BlockOptionParam);
                    }
                }
                Params::Bool(_) => return Err(Error::BoolParam),
                Params::None => Default::default(),
            };
            Ok(SubscriptionRequest::Logs(filter))
        }
        _ => Err(Error::OnlyLogAndNewHeadsSubscription),
    }
}
