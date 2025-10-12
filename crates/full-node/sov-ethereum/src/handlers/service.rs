use crate::handlers::ETH_RPC_ERROR;
use crate::to_jsonrpsee_error_object;
use crate::Cursor;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_consensus::BlockHeader;
use alloy_eips::eip1898::ParseBlockNumberError;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::BlockNumber;
use alloy_primitives::B256;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::{FilterBlockOption, Log};
use jsonrpsee::types::ErrorObjectOwned;
use sov_evm::{Evm, MaybeSealedBlock, Receipt, SealedBlock};
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
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
    #[error("Cursor not supported when filtering by block hash")]
    CursorNotSupportedForBlockHash,
    #[error("Too many logs in block {0}: limit is {1}")]
    TooManyLogsInBlock(B256, usize),
    #[error("Invalid cursor: block {block} starts at tx #{first_tx_idx}, which is greater than cursor tx #{cursor_tx_idx}.")]
    InvalidCursorTxIdx {
        block: BlockNumber,
        first_tx_idx: u64,
        cursor_tx_idx: u64,
    },
    #[error(
        "Invalid cursor: Cursor block number {cursor} should be within {from_block} and {to_block}."
    )]
    InvalidCursorBlockNumber {
        cursor: BlockNumber,
        from_block: BlockNumber,
        to_block: BlockNumber,
    },
    #[error(transparent)]
    ParseBlockNumber(ParseBlockNumberError),
}

impl From<Error> for ErrorObjectOwned {
    fn from(err: Error) -> ErrorObjectOwned {
        to_jsonrpsee_error_object(err.to_string(), ETH_RPC_ERROR)
    }
}

pub struct LogsService<S: Spec, Seq: Sequencer<Spec = S>> {
    filter: Filter,
    maybe_cursor: Option<Cursor>,
    max_logs: usize,
    state: ApiStateAccessor<S>,
    evm: Evm<S>,
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
        maybe_cursor: Option<Cursor>,
        max_logs: usize,
        state: ApiStateAccessor<S>,
    ) -> Self {
        Self {
            filter,
            maybe_cursor,
            max_logs,
            state,
            evm: Evm::<S>::default(),
            _phantom: PhantomData,
        }
    }

    pub async fn logs_for_filter(self) -> Result<LogsWithMaybeCursor, Error> {
        match self.filter.block_option {
            FilterBlockOption::AtBlockHash(block_hash) => self.by_hash(block_hash),
            FilterBlockOption::Range {
                from_block,
                to_block,
            } => self.by_range(from_block, to_block),
        }
    }

    pub fn by_hash(mut self, block_hash: B256) -> Result<LogsWithMaybeCursor, Error> {
        if self.maybe_cursor.is_some() {
            return Err(Error::CursorNotSupportedForBlockHash);
        }

        let Some(block_height) = self
            .evm
            .get_block_height_by_hash(&block_hash, &mut self.state)
        else {
            tracing::warn!(block_hash = %block_hash, "Block with hash not found");
            return Err(Error::BlockHashNotFound(block_hash));
        };

        let result = self.scan_block_range(block_height..=block_height)?;

        if result.cursor.is_some() {
            return Err(Error::TooManyLogsInBlock(block_hash, self.max_logs));
        }

        Ok(result)
    }

    pub fn by_range(
        mut self,
        from_block: Option<BlockNumberOrTag>,
        to_block: Option<BlockNumberOrTag>,
    ) -> Result<LogsWithMaybeCursor, Error> {
        let mut start = self.get_block_nr(from_block)?;
        let end = self.get_block_nr(to_block)?;
        let range = start..=end;
        if let Some(cursor) = self.maybe_cursor {
            if !range.contains(&cursor.block_height) {
                tracing::warn!(
                    cursor = cursor.block_height,
                    from_block = start,
                    to_block = end,
                    "Invalid cursor block height"
                );
                return Err(Error::InvalidCursorBlockNumber {
                    cursor: cursor.block_height,
                    from_block: start,
                    to_block: end,
                });
            }
            start = cursor.block_height;
        }
        self.scan_block_range(start..=end)
    }

    fn scan_block_range(
        &mut self,
        block_range: RangeInclusive<u64>,
    ) -> Result<LogsWithMaybeCursor, Error> {
        let mut rpc_logs = Vec::new();
        let mut cursor_indices = self.maybe_cursor.map(CursorIndices::new);

        for height in block_range {
            let next_cursor = self.logs_for_block(&mut rpc_logs, height, cursor_indices)?;

            cursor_indices = None;
            if let Some(cursor) = next_cursor {
                return Ok(LogsWithMaybeCursor::new(rpc_logs, Some(cursor.pack())));
            }
        }
        Ok(LogsWithMaybeCursor::new(rpc_logs, None))
    }

    /// Returns a cursor if the log limit was reached, otherwise None.
    /// Panics if a pending block is encountered (should be validated before calling).
    fn logs_for_block(
        &mut self,
        rpc_logs: &mut Vec<Log>,
        block_number: u64,
        indices_from_cursor: Option<CursorIndices>,
    ) -> Result<Option<Cursor>, Error> {
        let block = self.get_block(block_number)?;

        let header = &block.header;
        if !self.filter.matches_bloom(header.logs_bloom()) {
            return Ok(None);
        }

        let (tx_range, mut log_offset) =
            CursorIndices::tx_range_and_log_index(indices_from_cursor, &block)?;

        for tx_index in tx_range {
            let receipt = self.get_receipt(tx_index)?;
            let logs = receipt.receipt.logs;

            for (log_idx, log) in logs.into_iter().enumerate() {
                if log_idx < log_offset {
                    continue;
                }

                if rpc_logs.len() >= self.max_logs {
                    let cursor = Cursor {
                        block_height: block_number,
                        tx_index_absolute: tx_index,
                        log_index_in_tx: log_idx as u32,
                    };

                    return Ok(Some(cursor));
                }

                if self.filter.matches(&log) {
                    let rpc_log = Log {
                        inner: log,
                        block_hash: Some(header.hash()),
                        block_number: Some(receipt.block_number),
                        block_timestamp: Some(header.timestamp),
                        transaction_hash: Some(receipt.transaction_hash),
                        transaction_index: Some(receipt.transaction_index),
                        log_index: Some(receipt.log_index_start + log_idx as u64),
                        removed: false,
                    };
                    rpc_logs.push(rpc_log);
                }
            }
            log_offset = 0;
        }

        Ok(None)
    }

    fn get_receipt(&mut self, tx_idx: u64) -> Result<Receipt, Error> {
        self.evm.receipt(tx_idx, &mut self.state).ok_or_else(|| {
            tracing::error!(
                tx_idx,
                "Receipt for index not found. The state may have already been pruned."
            );
            Error::ReceiptPruned(tx_idx)
        })
    }

    fn get_block(&mut self, number: BlockNumber) -> Result<SealedBlock, Error> {
        let Some(block) = self.evm.get_maybe_sealed_block(number, &mut self.state) else {
            tracing::error!(
                number,
                "Block for height not found. The state may have already been pruned."
            );
            return Err(Error::BlockPruned(number));
        };
        let MaybeSealedBlock::Sealed(block) = block else {
            unreachable!("Pending blocks are not supported"); // This should be validated before calling this method.
        };
        Ok(block)
    }

    fn get_block_nr(
        &mut self,
        block_nr_or_tag: Option<BlockNumberOrTag>,
    ) -> Result<BlockNumber, Error> {
        let block_number = block_nr_or_tag.unwrap_or_default();
        let block_numbers = self.evm.block_numbers(&mut self.state);
        let block_number = match block_number {
            BlockNumberOrTag::Earliest => *block_numbers.start(),
            BlockNumberOrTag::Latest | BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => {
                *block_numbers.end()
            }
            BlockNumberOrTag::Number(nr) => nr,
            BlockNumberOrTag::Pending => return Err(Error::PendingBlock),
        };
        Ok(block_number)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorIndices {
    tx_index_absolute: u64,
    log_index_in_tx: u32,
}

impl CursorIndices {
    fn new(cursor: Cursor) -> Self {
        Self {
            tx_index_absolute: cursor.tx_index_absolute,
            log_index_in_tx: cursor.log_index_in_tx,
        }
    }

    fn tx_range_and_log_index(
        maybe_cursor_data: Option<Self>,
        block: &SealedBlock,
    ) -> Result<(Range<u64>, usize), Error> {
        let Some(cursor) = maybe_cursor_data else {
            return Ok((block.transactions.clone(), 0));
        };

        if block.transactions.start > cursor.tx_index_absolute {
            return Err(Error::InvalidCursorTxIdx {
                block: block.header.number,
                first_tx_idx: block.transactions.start,
                cursor_tx_idx: cursor.tx_index_absolute,
            });
        }

        let range = cursor.tx_index_absolute..block.transactions.end;
        Ok((range, (cursor.log_index_in_tx as usize)))
    }
}
