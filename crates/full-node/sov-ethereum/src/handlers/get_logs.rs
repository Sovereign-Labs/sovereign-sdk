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
use sov_evm::SealedBlock;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_rpc_eth_types::{FilterWithCursor, LogsWithMaybeCursor};
use sov_sequencer::SeqConfigExtension;
use std::ops::Range;
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
    logs_for_filter(parameters.one::<Filter>()?, None, ethereum)
        .await
        .map(|r| r.logs)
}

async fn logs_for_filter<S, Seq>(
    filter: Filter,
    maybe_cursor: Option<Cursor>,
    ethereum: Arc<Ethereum<S, Seq>>,
) -> Result<LogsWithMaybeCursor, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let state = &mut ethereum.api_state_accessor();

    match filter.block_option {
        FilterBlockOption::AtBlockHash(block_hash) => {
            logs_for_block_hash(filter, block_hash, &ethereum.extension, maybe_cursor, state)
        }
        FilterBlockOption::Range {
            from_block,
            to_block,
        } => logs_for_blocks_range(
            filter,
            from_block,
            to_block,
            &ethereum.extension,
            maybe_cursor,
            state,
        ),
    }
}

fn logs_for_block_hash<S>(
    filter: Filter,
    block_hash: B256,
    limits: &SeqConfigExtension,
    maybe_cursor: Option<Cursor>,
    state: &mut ApiStateAccessor<S>,
) -> Result<LogsWithMaybeCursor, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let evm = sov_evm::Evm::<S>::default();
    let mut rpc_logs = Vec::new();

    let block_height = match maybe_cursor {
        Some(cursor) => cursor.block_height as u64,
        None => {
            let Some(block_height) = evm.get_block_height_by_hash(&block_hash, state) else {
                let msg = format!("Block for block_hash {block_hash:?} does not exist");
                tracing::warn!(%msg);
                return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
            };

            block_height
        }
    };

    let next_cursor = logs_from_block(
        &mut rpc_logs,
        &filter,
        &evm,
        limits,
        block_height,
        maybe_cursor.map(CursorIndexses::new),
        state,
    )?;

    Ok(LogsWithMaybeCursor {
        logs: rpc_logs,
        cursor: next_cursor.map(|c| c.pack()),
    })
}

fn logs_for_blocks_range<S>(
    filter: Filter,
    from_block: Option<BlockNumberOrTag>,
    to_block: Option<BlockNumberOrTag>,
    limits: &SeqConfigExtension,
    mut maybe_cursor: Option<Cursor>,
    state: &mut ApiStateAccessor<S>,
) -> Result<LogsWithMaybeCursor, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let mut rpc_logs = Vec::new();
    let evm = sov_evm::Evm::<S>::default();

    let start = match maybe_cursor {
        Some(cursor) => cursor.block_height as u64,
        None => get_block_nr(from_block, &evm, state)?,
    };

    let end = get_block_nr(to_block, &evm, state)?;

    // We just validated that `start` and `end` are not pending.
    let block_range = RangeInclusive::new(start, end);

    for height in block_range {
        let next_cursor = logs_from_block(
            &mut rpc_logs,
            &filter,
            &evm,
            limits,
            height,
            maybe_cursor.map(CursorIndexses::new),
            state,
        )?;

        maybe_cursor = None;
        if next_cursor.is_some() {
            return Ok(LogsWithMaybeCursor {
                logs: rpc_logs,
                cursor: next_cursor.map(|c| c.pack()),
            });
        }
    }
    Ok(LogsWithMaybeCursor {
        logs: rpc_logs,
        cursor: None,
    })
}

// panics if a block number or pending block is passed.
fn logs_from_block<S>(
    rpc_logs: &mut Vec<Log>,
    filter: &Filter,
    evm: &sov_evm::Evm<S>,
    limits: &SeqConfigExtension,
    block_height: u64,
    indexses_from_cursor: Option<CursorIndexses>,
    state: &mut ApiStateAccessor<S>,
) -> Result<Option<Cursor>, ErrorObjectOwned>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let block = match evm.get_maybe_sealed_block(block_height, state) {
        Some(MaybeSealedBlock::Sealed(block)) => block,
        Some(MaybeSealedBlock::Pending(_)) => {
            // This should be validated before calling this method.
            panic!("Pending blocks are not supported")
        }
        None => {
            tracing::error!(
                block_height,
                "Block for height not found. The state may have already been pruned."
            );
            // This can happen if the state was pruned.
            let msg = format!(
                "Block for height {block_height:?} not found. The state may have already been pruned."
            );
            return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
        }
    };

    let header = &block.header;
    if !filter.matches_bloom(header.logs_bloom()) {
        return Ok(None);
    }
    let block_hash = header.hash();

    let (tx_range, mut next_log_index_in_tx) =
        CursorIndexses::tx_range_and_log_index(indexses_from_cursor, &block)?;

    for tx_index in tx_range {
        let Some(receipt) = evm.receipt(tx_index, state) else {
            // This can happen if the state was pruned.
            let msg = format!(
                "Receipt for index {tx_index:?} not found, The state may have already been pruned."
            );
            tracing::error!(tx_index, %block_hash, "Receipt for index not found, The state may have already been pruned.");
            return Err(to_jsonrpsee_error_object(&msg, ETH_RPC_ERROR));
        };

        let logs = receipt.receipt.logs;

        for (log_index_in_tx, log) in logs.into_iter().enumerate() {
            if log_index_in_tx < next_log_index_in_tx {
                continue;
            }

            if rpc_logs.len() >= limits.max_log_limit {
                let cursor = Cursor {
                    block_height: block_height as u32,
                    tx_index_absolute: tx_index,
                    log_index_in_tx: log_index_in_tx as u32,
                };

                return Ok(Some(cursor));
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
        next_log_index_in_tx = 0;
    }

    Ok(None)
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

pub async fn eth_get_logs_with_cursor<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<LogsWithMaybeCursor, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let FilterWithCursor { cursor, filter } = parameters.one::<FilterWithCursor>()?;
    let cursor = cursor.map(Cursor::unpack);
    logs_for_filter(filter, cursor, ethereum).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Cursor indicating where to start processing.
pub struct Cursor {
    /// Starting block height for this cursor (inclusive). Top 32 bits.
    pub block_height: u32,
    /// Absolute index of the first transaction to process (inclusive). Middle 64 bits.
    pub tx_index_absolute: u64,
    /// Index of the first log within that transaction to process (zero-based, inclusive). Lowest 32 bits.
    pub log_index_in_tx: u32,
}

impl Cursor {
    /// Packs `Self` to u128.
    pub fn pack(self) -> u128 {
        ((self.block_height as u128) << 96)
            | ((self.tx_index_absolute as u128) << 32)
            | (self.log_index_in_tx as u128)
    }

    /// Unpacks u128 to `Self`.
    pub fn unpack(v: u128) -> Self {
        let block_height = (v >> 96) as u32;
        let tx_index_absolute = ((v >> 32) & 0xFFFF_FFFF_FFFF_FFFFu128) as u64;
        let log_index_in_tx = (v & 0xFFFF_FFFFu128) as u32;

        Self {
            block_height,
            tx_index_absolute,
            log_index_in_tx,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorIndexses {
    tx_index_absolute: u64,
    log_index_in_tx: u32,
}

impl CursorIndexses {
    fn new(cursor: Cursor) -> Self {
        Self {
            tx_index_absolute: cursor.tx_index_absolute,
            log_index_in_tx: cursor.log_index_in_tx,
        }
    }

    fn tx_range_and_log_index(
        maybe_cursor_data: Option<Self>,
        block: &SealedBlock,
    ) -> Result<(Range<u64>, usize), ErrorObjectOwned> {
        let res = match maybe_cursor_data {
            Some(cursor) => {
                if block.transactions.start > cursor.tx_index_absolute {
                    let msg= format!(
                            "Invalid cursor: block {block} starts at tx #{block_first}, which is greater than cursor tx #{cursor_tx}.",
                            block = block.header.number,
                            block_first = block.transactions.start,
                            cursor_tx = cursor.tx_index_absolute,
                    );

                    return Err(to_jsonrpsee_error_object(msg, ETH_RPC_ERROR));
                }

                (
                    Range {
                        start: cursor.tx_index_absolute,
                        end: block.transactions.end,
                    },
                    (cursor.log_index_in_tx as usize),
                )
            }
            None => (block.transactions.clone(), 0),
        };
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packs_and_unpacks_exact_layout() {
        let c = Cursor {
            block_height: 0x89AB_CDEF,
            tx_index_absolute: 0x0123_4567_89AB_CDEF,
            log_index_in_tx: 0x7654_3210,
        };

        let packed = c.pack();
        assert_eq!(packed, 0x89AB_CDEF_0123_4567_89AB_CDEF_7654_3210u128);

        let unpacked = Cursor::unpack(packed);
        assert_eq!(unpacked, c);
    }

    #[test]
    fn test_roundtrip() {
        let samples = [
            Cursor {
                block_height: 0,
                tx_index_absolute: 0,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: u32::MAX,
                tx_index_absolute: 0,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: 0,
                tx_index_absolute: u64::MAX,
                log_index_in_tx: 0,
            },
            Cursor {
                block_height: 0,
                tx_index_absolute: 0,
                log_index_in_tx: u32::MAX,
            },
            Cursor {
                block_height: u32::MAX,
                tx_index_absolute: u64::MAX,
                log_index_in_tx: u32::MAX,
            },
            Cursor {
                block_height: 1,
                tx_index_absolute: 2,
                log_index_in_tx: 3,
            },
            Cursor {
                block_height: 0x0123_4567,
                tx_index_absolute: 0x89AB_CDEF_FEDC_BA98,
                log_index_in_tx: 0x7654_3210,
            },
        ];

        for c in samples {
            let packed = c.pack();
            let unpacked = Cursor::unpack(packed);
            assert_eq!(unpacked, c);
        }
    }
}
