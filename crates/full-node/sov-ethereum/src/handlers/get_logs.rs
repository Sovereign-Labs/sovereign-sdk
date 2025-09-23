#![allow(dead_code)]
use crate::handlers::ETH_RPC_ERROR;
use crate::to_jsonrpsee_error_object;
use crate::Ethereum;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::B256;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::FilterBlockOption;
use alloy_rpc_types::Log;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_evm::MaybeSealedBlock;
use sov_evm::PendingOrBlock;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_sequencer::SeqConfigExtension;
use std::ops::RangeInclusive;
use std::sync::Arc;

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
    logs_for_filter(parameters.one::<Filter>()?, ethereum).await
}

async fn logs_for_filter<S, Seq>(
    filter: Filter,
    ethereum: Arc<Ethereum<S, Seq>>,
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
            logs_for_block_hash(filter, block_hash, &ethereum.extension, state)
        }
        FilterBlockOption::Range {
            from_block,
            to_block,
        } => logs_for_blocks_range(filter, from_block, to_block, &ethereum.extension, state),
    }
}

fn logs_for_block_hash<S>(
    filter: Filter,
    block_hash: B256,
    limits: &SeqConfigExtension,
    state: &mut ApiStateAccessor<S>,
) -> Result<Vec<Log>, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let evm = sov_evm::Evm::<S>::default();
    let mut rpc_logs = Vec::new();

    let Some(height) = evm.get_block_height_by_hash(&block_hash, state) else {
        let msg = format!("Block for block_hash {block_hash:?} does not exist");
        tracing::warn!(%msg);
        return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
    };
    logs_from_block(&mut rpc_logs, height, &filter, &evm, limits, state)?;

    Ok(rpc_logs)
}

fn logs_for_blocks_range<S>(
    filter: Filter,
    from_block: Option<BlockNumberOrTag>,
    to_block: Option<BlockNumberOrTag>,
    limits: &SeqConfigExtension,
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

    // We jsut validated that `start` and `end` are not peneding.
    let block_range = RangeInclusive::new(start, end);
    for height in block_range {
        let should_contnue = logs_from_block(&mut rpc_logs, height, &filter, &evm, limits, state)?;
        if !should_contnue {
            break;
        }
    }
    Ok(rpc_logs)
}

// anics if a block number or pending block is passed.
fn logs_from_block<S>(
    rpc_logs: &mut Vec<Log>,
    height: u64,
    filter: &Filter,
    evm: &sov_evm::Evm<S>,
    limits: &SeqConfigExtension,
    state: &mut ApiStateAccessor<S>,
) -> Result<bool, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let block = match evm.get_maybe_sealed_block(height, state) {
        Some(MaybeSealedBlock::Sealed(block)) => block,
        Some(MaybeSealedBlock::Pending(_)) => {
            // This should be validate before calling this method.
            panic!("Pending blocks are not supported")
        }
        None => {
            // This can hapen if the state was pruned.
            let msg = format!(
                "Block for height {height:?} not found. The state may have already been pruned.",
            );
            return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
        }
    };

    let header = block.header;
    if !filter.matches_bloom(header.logs_bloom()) {
        return Ok(true);
    }
    let block_hash = header.hash();

    for index in block.transactions {
        let Some(receipt) = evm.receipt(index, state) else {
            // This can hapen if the state was pruned.
            let msg = format!(
                "Receipt for index {index:?} not found, The state may have already been pruned."
            );
            tracing::error!(%msg);
            return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
        };

        let logs = receipt.receipt.logs;

        for (log_index_in_tx, log) in logs.into_iter().enumerate() {
            if rpc_logs.len() >= limits.max_log_limit {
                return Ok(false);
            }

            if filter.matches(&log) {
                let rpc_log = Log {
                    inner: log,
                    block_hash: Some(block_hash),
                    block_number: Some(receipt.block_number),
                    block_timestamp: Some(header.timestamp),
                    transaction_hash: Some(receipt.transaction_hash),
                    transaction_index: Some(receipt.transaction_index),
                    log_index: Some(receipt.log_index_start + log_index_in_tx as u64),
                    removed: false,
                };
                rpc_logs.push(rpc_log);
            }
        }
    }

    Ok(true)
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
        PendingOrBlock::Pending => Err(to_jsonrpsee_error_object(
            "Pending blocks are not supported",
            ETH_RPC_ERROR,
        )),
        PendingOrBlock::Invalid(err) => {
            let msg = format!("Invalid block: {err}");
            Err(to_jsonrpsee_error_object(msg, ETH_RPC_ERROR))
        }
        PendingOrBlock::Number(number) => Ok(number),
    }
}
