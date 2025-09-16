use crate::Ethereum;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_primitives::B256;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_modules_api::Spec;
use std::sync::Arc;

pub async fn eth_get_logs<S, Seq>(
    _parameters: JRpcParams<'static>,
    _ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<B256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    todo!()
}
