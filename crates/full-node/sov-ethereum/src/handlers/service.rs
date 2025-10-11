use crate::handlers::ETH_RPC_ERROR;
use crate::to_jsonrpsee_error_object;
use crate::Cursor;
use crate::EthereumAddress;
use crate::EthereumAuthenticator;
use crate::FromVmAddress;
use crate::HasKernel;
use crate::Sequencer;
use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::BlockNumber;
use alloy_primitives::B256;
use alloy_rpc_types::eth::Filter;
use alloy_rpc_types::{FilterBlockOption, Log};
use jsonrpsee::types::ErrorObjectOwned;
use sov_evm::MaybeSealedBlock;
use sov_evm::PendingOrBlock;
use sov_evm::SealedBlock;
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::Spec;
use sov_rpc_eth_types::LogsWithMaybeCursor;
use sov_sequencer::SeqConfigExtension;
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
    #[error("Invalid cursor: block {block} starts at tx #{first_tx_idx}, which is greater than cursor tx #{cursor_tx_idx}.")]
    InvalidCursor {
        block: BlockNumber,
        first_tx_idx: u64,
        cursor_tx_idx: u64,
    },
}

impl From<Error> for ErrorObjectOwned {
    fn from(err: Error) -> ErrorObjectOwned {
        to_jsonrpsee_error_object(err.to_string(), ETH_RPC_ERROR)
    }
}

pub struct LogsService<S: Spec, Seq: Sequencer<Spec = S>> {
    filter: Filter,
    maybe_cursor: Option<Cursor>,
    limits: SeqConfigExtension,
    state: ApiStateAccessor<S>,
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
        limits: SeqConfigExtension,
        state: ApiStateAccessor<S>,
    ) -> Self {
        Self {
            filter,
            maybe_cursor,
            limits,
            state,
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

        let evm = sov_evm::Evm::<S>::default();
        let mut rpc_logs = Vec::new();

        let Some(block_height) = evm.get_block_height_by_hash(&block_hash, &mut self.state)
        else {
            tracing::warn!(block_hash = %block_hash, "Block with hash not found");
            return Err(Error::BlockHashNotFound(block_hash));
        };

        let next_cursor = self.logs_for_block(
            &mut rpc_logs,
            &evm,
            block_height,
            None,
        )?;

        Ok(LogsWithMaybeCursor {
            logs: rpc_logs,
            cursor: next_cursor.map(|c| c.pack()),
        })
    }

    pub fn by_range(
        mut self,
        from_block: Option<BlockNumberOrTag>,
        to_block: Option<BlockNumberOrTag>,
    ) -> Result<LogsWithMaybeCursor, Error> {
        let mut rpc_logs = Vec::new();
        let evm = sov_evm::Evm::<S>::default();

        let start = match self.maybe_cursor {
            Some(cursor) => cursor.block_height,
            None => get_block_nr(from_block, &evm, &mut self.state)?,
        };

        let end = get_block_nr(to_block, &evm, &mut self.state)?;

        // We just validated that `start` and `end` are not pending.
        let block_range = RangeInclusive::new(start, end);

        for height in block_range {
            let next_cursor = self.logs_for_block(
                &mut rpc_logs,
                &evm,
                height,
                self.maybe_cursor.map(CursorIndices::new),
            )?;

            self.maybe_cursor = None;
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

    /// Returns a cursor if the log limit was reached, otherwise None.
    /// Panics if a pending block is encountered (should be validated before calling).
    fn logs_for_block(
        &mut self,
        rpc_logs: &mut Vec<Log>,
        evm: &sov_evm::Evm<S>,
        block_number: u64,
        indices_from_cursor: Option<CursorIndices>,
    ) -> Result<Option<Cursor>, Error> {
        let block = match evm.get_maybe_sealed_block(block_number, &mut self.state) {
            Some(MaybeSealedBlock::Sealed(block)) => block,
            Some(MaybeSealedBlock::Pending(_)) => unreachable!("Pending blocks are not supported"), // This should be validated before calling this method.
            None => {
                tracing::error!(
                    block_number,
                    "Block for height not found. The state may have already been pruned."
                );
                return Err(Error::BlockPruned(block_number));
            }
        };

        let header = &block.header;
        if !self.filter.matches_bloom(header.logs_bloom()) {
            return Ok(None);
        }
        let block_hash = header.hash();

        let (tx_range, mut next_log_index_in_tx) =
            CursorIndices::tx_range_and_log_index(indices_from_cursor, &block)?;

        for tx_index in tx_range {
            let Some(receipt) = evm.receipt(tx_index, &mut self.state) else {
                tracing::error!(tx_index, %block_hash, "Receipt for index not found. The state may have already been pruned.");
                return Err(Error::ReceiptPruned(tx_index));
            };

            let logs = receipt.receipt.logs;

            for (log_index_in_tx, log) in logs.into_iter().enumerate() {
                if log_index_in_tx < next_log_index_in_tx {
                    continue;
                }

                if rpc_logs.len() >= self.limits.max_log_limit {
                    let cursor = Cursor {
                        block_height: block_number,
                        tx_index_absolute: tx_index,
                        log_index_in_tx: log_index_in_tx as u32,
                    };

                    return Ok(Some(cursor));
                }

                if self.filter.matches(&log) {
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
}

fn get_block_nr<S>(
    block_nr_or_tag: Option<BlockNumberOrTag>,
    evm: &sov_evm::Evm<S>,
    state: &mut ApiStateAccessor<S>,
) -> Result<u64, Error>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    let block_num = block_nr_or_tag.map(|b| b.to_string());
    let number = evm.str_to_block_nr(block_num, state);
    match number {
        PendingOrBlock::Pending => Err(Error::PendingBlock),
        PendingOrBlock::Invalid(err) => Err(Error::InvalidBlock(err)),
        PendingOrBlock::Number(number) => Ok(number),
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
            return Err(Error::InvalidCursor {
                block: block.header.number,
                first_tx_idx: block.transactions.start,
                cursor_tx_idx: cursor.tx_index_absolute,
            });
        }

        let range = cursor.tx_index_absolute..block.transactions.end;
        Ok((range, (cursor.log_index_in_tx as usize)))
    }
}
