#![allow(dead_code)]
use crate::handlers::ETH_RPC_ERROR;
use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::B256;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::FilterBlockOption;
use alloy_rpc_types::Log;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_evm::PendingOrBlock;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use std::ops::RangeInclusive;
use std::sync::Arc;

#[derive(Default, Debug, Clone, Copy)]
pub struct QueryLimits {
    /// Maximum number of blocks that could be scanned per filter
    pub max_blocks_per_filter: Option<u64>,
    /// Maximum number of logs that can be returned in a response
    pub max_logs_per_response: Option<usize>,
}

impl QueryLimits {
    /// Construct an object with no limits (more explicit than using default constructor)
    pub fn no_limits() -> Self {
        Default::default()
    }
}

pub async fn eth_get_logs<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<Vec<Log>, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    logs_for_filter(
        parameters.one::<Filter>()?,
        ethereum,
        QueryLimits::no_limits(),
    )
    .await
}

async fn logs_for_filter<S, Seq>(
    filter: Filter,
    ethereum: Arc<Ethereum<S, Seq>>,
    _limits: QueryLimits,
) -> Result<Vec<Log>, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let state = &mut ethereum.api_state_accessor();

    match filter.block_option {
        FilterBlockOption::AtBlockHash(block_hash) => {
            logs_for_block_hash(filter, block_hash, state)
        }
        FilterBlockOption::Range {
            from_block,
            to_block,
        } => logs_for_blocks_range(filter, from_block, to_block, state),
    }
}

fn logs_for_block_hash<S>(
    filter: Filter,
    block_hash: B256,
    state: &mut ApiStateAccessor<S>,
) -> Result<Vec<Log>, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let evm = sov_evm::Evm::<S>::default();
    let mut rpc_logs = Vec::new();

    let Some(height) = evm.get_block_height_by_hash(&block_hash, state) else {
        let msg = format!("Block for block_hash {:?} does not exist", block_hash);
        tracing::warn!(%msg);
        return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
    };
    logs_from_block(&mut rpc_logs, height, &filter, &evm, state)?;

    Ok(rpc_logs)
}

fn logs_for_blocks_range<S>(
    filter: Filter,
    from_block: Option<BlockNumberOrTag>,
    to_block: Option<BlockNumberOrTag>,
    state: &mut ApiStateAccessor<S>,
) -> Result<Vec<Log>, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let mut rpc_logs = Vec::new();
    let evm = sov_evm::Evm::<S>::default();

    let start = get_block_nr(from_block, &evm, state)?;
    let end = get_block_nr(to_block, &evm, state)?;

    // TODO bloom filters
    let block_range = RangeInclusive::new(start, end);
    for height in block_range {
        logs_from_block(&mut rpc_logs, height, &filter, &evm, state)?;
    }
    Ok(rpc_logs)
}

fn logs_from_block<S>(
    rpc_logs: &mut Vec<Log>,
    height: u64,
    filter: &Filter,
    evm: &sov_evm::Evm<S>,
    state: &mut ApiStateAccessor<S>,
) -> Result<(), ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let block = evm.get_maybe_sealed_block(height, state);

    for index in block.transactions_start()..block.transactions_end() {
        let Some(receipt) = evm.receipt(index, state) else {
            // This can hapen if the state was pruned.
            let msg = format!("Receipt for index {:?} does not exist", index);
            tracing::error!(%msg);
            return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
        };

        let logs = receipt.receipt.logs;

        for (log_index_in_tx, log) in logs.into_iter().enumerate() {
            if filter.matches(&log) {
                let rpc_log = Log {
                    inner: log,
                    block_hash: block.hash(),
                    block_number: Some(receipt.block_number),
                    block_timestamp: Some(block.timestamp()),
                    transaction_hash: Some(receipt.transaction_hash),
                    transaction_index: Some(receipt.transaction_index),
                    log_index: Some(receipt.log_index_start + log_index_in_tx as u64),
                    removed: false,
                };
                rpc_logs.push(rpc_log);
            }
        }
    }

    Ok(())
}

fn get_block_nr<S>(
    block_nr_or_tag: Option<BlockNumberOrTag>,
    evm: &sov_evm::Evm<S>,
    state: &mut ApiStateAccessor<S>,
) -> Result<u64, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let number = evm.str_to_block_nr(block_nr_or_tag.map(|b| b.to_string()), state);
    match number {
        PendingOrBlock::Pending => {
            return Err(to_jsonrpsee_error_object(
                "Pending blocks are not supported",
                ETH_RPC_ERROR,
            ))
        }
        PendingOrBlock::Invalid(err) => {
            let msg = format!("Invalid block: {err}");
            return Err(to_jsonrpsee_error_object(msg, ETH_RPC_ERROR));
        }
        PendingOrBlock::Number(number) => Ok(number),
    }
}
