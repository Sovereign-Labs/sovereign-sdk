use super::cursor::Cursor;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use crate::{rpc_invalid_params, rpc_limit_exceeded, rpc_resource_not_found};
use alloy_consensus::BlockHeader;
use alloy_consensus::TxReceipt;
use alloy_eips::eip1898::ParseBlockNumberError;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::BlockHash;
use alloy_primitives::BlockNumber;
use alloy_primitives::B256;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::{FilterBlockOption, Log};
use derive_more::{Deref, From};
use jsonrpsee::types::ErrorObjectOwned;
use sov_evm::{Evm, MaybeSealedBlock, Receipt};
use sov_modules_api::da::Time;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_rpc_eth_types::rpc_error_with_code;
use sov_rpc_eth_types::LogWithExecutionTimestamp;
use sov_rpc_eth_types::LogsWithMaybeCursor;
use std::marker::PhantomData;
use std::ops::Range;
use std::ops::RangeInclusive;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Receipt for index {0} not found. The state may have already been pruned.")]
    ReceiptPruned(u64),
    #[error("Pending blocks are not supported")]
    PendingBlock,
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("Block for height {0} not found. The state may have already been pruned.")]
    BlockPruned(BlockNumber),
    #[error("Block with hash {0} not found")]
    BlockHashNotFound(B256),
    #[error("Too many logs in block {0}: limit is {1}")]
    TooManyLogsInBlock(B256, usize),
    #[error(
        "Invalid cursor: Cursor block number {cursor} should be within [{}..{}].", range.start(), range.end()
    )]
    InvalidCursorBlockNumber {
        cursor: BlockNumber,
        range: RangeInclusive<BlockNumber>,
    },
    #[error("Invalid cursor: Cursor tx index {cursor} should be within [{}..{}).", range.start, range.end)]
    InvalidCursorTxIdx { cursor: u64, range: Range<u64> },
    #[error("Invalid cursor: Cursor log index {cursor} should be within [{}..{}).", range.start, range.end)]
    InvalidCursorLogIdx { cursor: u32, range: Range<u32> },
    #[error(transparent)]
    ParseBlockNumber(ParseBlockNumberError),
}
type Result<T> = std::result::Result<T, Error>;

impl From<Error> for ErrorObjectOwned {
    fn from(err: Error) -> ErrorObjectOwned {
        match err {
            Error::ReceiptPruned(_) | Error::BlockPruned(_) => {
                rpc_error_with_code(4444, err.to_string())
            }
            Error::BlockHashNotFound(_)
            | Error::InvalidCursorBlockNumber { .. }
            | Error::InvalidCursorTxIdx { .. }
            | Error::InvalidCursorLogIdx { .. } => rpc_resource_not_found(err.to_string()),
            Error::TooManyLogsInBlock(_, _) => rpc_limit_exceeded(err.to_string()),
            Error::PendingBlock | Error::InvalidBlock(_) | Error::ParseBlockNumber(_) => {
                rpc_invalid_params(err.to_string())
            }
        }
    }
}

/// A container for values that can only be deref'd immutably.
#[derive(From, Deref)]
struct Immutable<T>(T);

pub struct LogsService<S: Spec, Seq: Sequencer<Spec = S>> {
    filter: Immutable<Filter>,
    cursor: Immutable<Option<Cursor>>,
    max_logs: Immutable<usize>,
    evm: Evm<S>,
    logs: Vec<LogWithExecutionTimestamp>,
    logs_serialized_size: usize,
    state: ApiStateAccessor<S>,
    response_size_limit: Immutable<usize>,
    _phantom: PhantomData<(S, Seq)>,
}

impl<S, Seq> LogsService<S, Seq>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    pub fn new(
        filter: Filter,
        cursor: Option<Cursor>,
        max_logs: usize,
        state: ApiStateAccessor<S>,
        response_size_limit: usize,
    ) -> Self {
        Self {
            filter: filter.into(),
            cursor: cursor.into(),
            max_logs: max_logs.into(),
            state,
            evm: Evm::<S>::default(),
            logs: vec![],
            logs_serialized_size: 0,
            response_size_limit: response_size_limit.into(),
            _phantom: PhantomData,
        }
    }

    pub async fn logs_for_filter(self) -> Result<LogsWithMaybeCursor> {
        match self.filter.block_option {
            FilterBlockOption::AtBlockHash(block_hash) => self.by_hash(block_hash),
            FilterBlockOption::Range {
                from_block,
                to_block,
            } => self.by_range(from_block, to_block),
        }
    }

    fn by_hash(mut self, block_hash: B256) -> Result<LogsWithMaybeCursor> {
        let block_height = self.resolve_block_hash(block_hash)?;
        let maybe_cursor = self.scan_block_range(block_height..=block_height)?;
        Ok(LogsWithMaybeCursor::new(
            self.logs,
            maybe_cursor.map(|c| c.pack()),
        ))
    }

    fn by_range(
        mut self,
        from_block: Option<BlockNumberOrTag>,
        to_block: Option<BlockNumberOrTag>,
    ) -> Result<LogsWithMaybeCursor> {
        let start = self.get_block_nr(from_block)?;
        let end = self.get_block_nr(to_block)?;
        let maybe_cursor = self.scan_block_range(start..=end)?;
        Ok(LogsWithMaybeCursor::new(
            self.logs,
            maybe_cursor.map(|c| c.pack()),
        ))
    }

    fn apply_block_level_cursor(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> Result<RangeInclusive<u64>> {
        if let Some(cursor) = *self.cursor {
            if !range.contains(&cursor.block_height) {
                tracing::warn!(
                    cursor = cursor.block_height,
                    ?range,
                    "Invalid cursor block height"
                );
                return Err(Error::InvalidCursorBlockNumber {
                    cursor: cursor.block_height,
                    range,
                });
            }
            return Ok(cursor.block_height..=*range.end());
        };
        Ok(range)
    }

    fn scan_block_range(&mut self, range: RangeInclusive<u64>) -> Result<Option<Cursor>> {
        let range = self.apply_block_level_cursor(range)?;
        for block_number in range {
            let block = self.get_block(block_number)?;
            if !self.filter.matches_bloom(block.header().logs_bloom()) {
                continue;
            }
            if let Some(cursor) = self.scan_block(block)? {
                return Ok(Some(cursor));
            }
        }
        Ok(None)
    }

    fn apply_tx_level_cursor(
        &self,
        tx_range_absolut: Range<u64>,
        block_number: BlockNumber,
    ) -> Result<Range<u64>> {
        if let Some(cursor) = *self.cursor {
            if cursor.block_height == block_number {
                if !tx_range_absolut.contains(&cursor.tx_index_absolute) {
                    tracing::warn!(
                        cursor = cursor.tx_index_absolute,
                        ?tx_range_absolut,
                        "Invalid cursor tx index"
                    );
                    return Err(Error::InvalidCursorTxIdx {
                        cursor: cursor.tx_index_absolute,
                        range: tx_range_absolut,
                    });
                }
                return Ok(cursor.tx_index_absolute..tx_range_absolut.end);
            }
        }
        Ok(tx_range_absolut)
    }

    fn scan_block(&mut self, block: MaybeSealedBlock) -> Result<Option<Cursor>> {
        let mut tx_range_absolut = block.tx_range();
        tx_range_absolut = self.apply_tx_level_cursor(tx_range_absolut, block.number())?;
        for tx_idx_absolute in tx_range_absolut {
            let (receipt, time) = self.get_receipt(tx_idx_absolute)?;
            if !self.filter.matches_bloom(receipt.bloom()) {
                continue;
            }
            if let Some(cursor) = self.scan_tx(tx_idx_absolute, receipt, &block, time)? {
                return Ok(Some(cursor));
            }
        }

        Ok(None)
    }

    fn apply_log_level_cursor(
        &self,
        log_range: Range<u32>,
        tx_index_absolute: u64,
        block_number: u64,
    ) -> Result<u32> {
        if let Some(cursor) = *self.cursor {
            if cursor.block_height == block_number && cursor.tx_index_absolute == tx_index_absolute
            {
                if !log_range.contains(&cursor.log_index_in_tx) {
                    tracing::warn!(
                        cursor = cursor.block_height,
                        ?log_range,
                        "Invalid cursor log index"
                    );
                    return Err(Error::InvalidCursorLogIdx {
                        cursor: cursor.log_index_in_tx,
                        range: log_range,
                    });
                }
                return Ok(cursor.log_index_in_tx);
            }
        }
        Ok(0)
    }

    fn scan_tx(
        &mut self,
        tx_index_absolute: u64,
        receipt: Receipt,
        block: &MaybeSealedBlock,
        time: Time,
    ) -> Result<Option<Cursor>> {
        let header = block.header();
        let logs = receipt.receipt.logs;
        let log_range = 0_u32..(logs.len() as u32);
        let logs_iter = logs.into_iter().enumerate();
        let skipped_logs =
            self.apply_log_level_cursor(log_range, tx_index_absolute, header.number())?;

        // As logs iter is pre-enumerated - we keep correct indices
        for (idx, log) in logs_iter.skip(skipped_logs as usize) {
            if self.logs.len() >= *self.max_logs {
                return Ok(Some(Cursor {
                    block_height: header.number(),
                    tx_index_absolute,
                    log_index_in_tx: idx as u32,
                }));
            }
            if !self.filter.matches(&log) {
                continue;
            }
            let rpc_log = LogWithExecutionTimestamp {
                log: Log {
                    inner: log,
                    block_hash: block.hash(),
                    block_number: Some(receipt.block_number),
                    block_timestamp: Some(block.timestamp()),
                    transaction_hash: Some(receipt.transaction_hash),
                    transaction_index: Some(receipt.transaction_index),
                    log_index: Some(receipt.log_index_start + idx as u64),
                    removed: false,
                },
                time_executed_ms: time.as_millis().try_into().unwrap_or_default(),
            };
            let log_size = serialized_size(&rpc_log);
            if self.logs_serialized_size + log_size >= *self.response_size_limit {
                return Ok(Some(Cursor {
                    block_height: header.number(),
                    tx_index_absolute,
                    log_index_in_tx: idx as u32,
                }));
            }
            self.logs_serialized_size += log_size;
            self.logs.push(rpc_log);
        }
        Ok(None)
    }

    fn get_receipt(&mut self, tx_idx: u64) -> Result<(Receipt, Time)> {
        self.evm.receipt(tx_idx, &mut self.state).ok_or_else(|| {
            tracing::error!(
                tx_idx,
                "Receipt for index not found. The state may have already been pruned."
            );
            Error::ReceiptPruned(tx_idx)
        })
    }

    fn get_block(&mut self, number: BlockNumber) -> Result<MaybeSealedBlock> {
        let Some(block) = self.evm.get_maybe_sealed_block(number, &mut self.state) else {
            tracing::error!(
                number,
                "Block for height not found. The state may have already been pruned."
            );
            return Err(Error::BlockPruned(number));
        };
        Ok(block)
    }

    fn resolve_block_hash(&mut self, block_hash: BlockHash) -> Result<u64> {
        let Some(block_height) = self.evm.block_height(&block_hash, &mut self.state) else {
            tracing::warn!(block_hash = %block_hash, "Block with hash not found");
            return Err(Error::BlockHashNotFound(block_hash));
        };
        Ok(block_height)
    }

    fn get_block_nr(&mut self, block_nr_or_tag: Option<BlockNumberOrTag>) -> Result<BlockNumber> {
        let block_number = block_nr_or_tag.unwrap_or_default();
        Ok(self.evm.resolve_block_number(block_number, &mut self.state))
    }
}

fn serialized_size(log: &LogWithExecutionTimestamp) -> usize {
    b"\"{address\":".len() + 44 // 20 byte address, hex-encoded + open/close quotes and 0x
        + b",\"data\":".len() + 4 + log.log.inner.data.data.len() * 2 // 32 byte data, hex-encoded + open/close quotes and 0x + quotation marks
        + b",\"topics\":[]".len() + log.log.inner.topics().len() * 68 // 32 byte topic, hex-encoded + open/close quotes and 0x
        + b",\"blockHash\":".len() + 68 // Block hash (0x-prefixed + 32 data bytes) + quotation marks
        + b",\"transactionHash\":".len() + 68 // Transaction hash (0x-prefixed + 32 data bytes) + quotation mark
        + b",\"blockNumber\":".len() + 14 // Assume a billion blocks (plus open/close quotes and 0x prefix)
        + b",\"blockTimestamp\":".len() + 16 // Conservatively assume a long time (current unix timestamp is only 8 hex digits, this assumes 12)
        + b",\"timeExecutedMs\":".len() + 19 // Time executed in milliseconds
        + b",\"transactionIndex\":".len() + 12 // Conservatively assume millions of txs per block (plus open/close quotes and 0x prefix)    
        + b",\"logIndex\":".len() + 14 // Conservatively assume 256 logs per tx
        + b",\"removed\":false}".len() // false is longer than true
                                       // See example serialized log below:
                                       // r#"{"address":"0x0000000000000000000000000000000000000069","topics":["0x0000000000000000000000000000000000000000000000000000000000000069"],"data":"0x69","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000069","blockNumber":"0x69","blockTimestamp":"0x69","transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000069","transactionIndex":"0x69","logIndex":"0x69","removed":false}"#
}
