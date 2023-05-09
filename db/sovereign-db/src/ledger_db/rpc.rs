use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sovereign_sdk::{
    rpc::{
        BatchIdentifier, EventIdentifier, LedgerRpcProvider, QueryMode, SlotIdentifier,
        TxIdentifier,
    },
    stf::Event,
};

use crate::schema::{
    tables::{
        BatchByHash, BatchByNumber, EventByNumber, SlotByHash, SlotByNumber, TxByHash, TxByNumber,
    },
    types::{
        BatchNumber, EventNumber, SlotNumber, Status, StoredBatch, StoredSlot, StoredTransaction,
        TxNumber,
    },
};

use super::LedgerDB;

#[derive(Debug, PartialEq, Eq, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemOrHash<T> {
    Hash([u8; 32]),
    Full(T),
}

#[derive(Debug, PartialEq, Eq, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct SlotResponse {
    pub number: u64,
    pub hash: [u8; 32],
    pub batch_range: std::ops::Range<BatchNumber>,
    pub batches: Option<Vec<ItemOrHash<BatchResponse>>>,
}

#[derive(Debug, PartialEq, Eq, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct BatchResponse {
    pub hash: [u8; 32],
    pub tx_range: std::ops::Range<TxNumber>,
    pub txs: Option<Vec<ItemOrHash<TxResponse>>>,
    pub status: Status,
}

#[derive(Debug, PartialEq, Eq, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TxResponse {
    pub hash: [u8; 32],
    pub event_range: std::ops::Range<EventNumber>,
    pub body: Option<Vec<u8>>,
    pub status: Status,
}

struct QueriedRange {
    pub inner: std::ops::Range<SlotNumber>,
    pub original_indices: Vec<usize>,
}

impl From<StoredTransaction> for TxResponse {
    fn from(tx: StoredTransaction) -> Self {
        Self {
            hash: tx.hash,
            event_range: tx.events,
            body: Some(tx.data.as_ref().to_vec()),
            status: tx.status,
        }
    }
}

impl LedgerRpcProvider for LedgerDB {
    type SlotResponse = SlotResponse;

    type BatchResponse = BatchResponse;

    type TxResponse = TxResponse;

    type EventResponse = Event;

    fn get_slots(
        &self,
        slot_ids: &[sovereign_sdk::rpc::SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::SlotResponse>>, anyhow::Error> {
        // TODO: https://github.com/Sovereign-Labs/sovereign/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually

        // First resolve the slot identifiers and store the slot numbers
        let slot_nums_result: Result<Vec<(usize, Option<SlotNumber>)>, anyhow::Error> = slot_ids
            .iter()
            .enumerate()
            .map(
                |(idx, slot_id)| match self.resolve_slot_identifier(slot_id) {
                    Ok(slot_number) => Ok((idx, slot_number)),
                    Err(e) => Err(e),
                },
            )
            .collect();

        let mut slot_nums = match slot_nums_result {
            Ok(slot_numbers) => slot_numbers,
            Err(e) => return Err(e),
        };

        // Then sort the slot numbers by requested order
        slot_nums.sort_unstable_by(|(_, slot_id_a), (_, slot_id_b)| slot_id_a.cmp(slot_id_b));

        // Then identify sequential ranges
        let mut slot_ranges: Vec<QueriedRange> = Vec::new();
        let mut current_range: Option<QueriedRange> = None;

        for (orig_idx, slot_num_opt) in slot_nums {
            if let Some(num) = slot_num_opt {
                match current_range {
                    Some(range) if SlotNumber(Into::<u64>::into(range.inner.end) + 1) == num => {
                        current_range = Some(QueriedRange {
                            inner: range.inner.start..num,
                            original_indices: range.original_indices,
                        });
                    }
                    _ => {
                        if let Some(range) = current_range {
                            slot_ranges.push(range);
                        }
                        current_range = Some(QueriedRange {
                            inner: num..SlotNumber(Into::<u64>::into(num) + 1),
                            original_indices: vec![orig_idx],
                        });
                    }
                }
            }
        }

        // For each identified range, make a single query to fetch tx's
        // From the corresponding blocks, then store into result vector.
        let mut out = Vec::with_capacity(slot_ids.len());
        for slot_range in slot_ranges {
            let stored_slots = self.get_slot_range(&slot_range.inner)?;

            for ((queried_slot, original_idx), slot_num) in stored_slots
                .into_iter()
                .zip(slot_range.original_indices)
                .zip(slot_range.inner.start.into()..slot_range.inner.end.into())
            {
                out[original_idx] =
                    Some(self.populate_slot_response(slot_num, queried_slot, query_mode)?);
            }
        }
        Ok(out)
    }

    fn get_batches(
        &self,
        batch_ids: &[sovereign_sdk::rpc::BatchIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::BatchResponse>>, anyhow::Error> {
        // TODO: https://github.com/Sovereign-Labs/sovereign/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually

        let mut out = Vec::with_capacity(batch_ids.len());
        for batch_id in batch_ids {
            let batch_num = self.resolve_batch_identifier(batch_id)?;
            out.push(match batch_num {
                Some(num) => {
                    if let Some(stored_batch) = self.db.get::<BatchByNumber>(&num)? {
                        Some(self.populate_batch_response(stored_batch, query_mode)?)
                    } else {
                        None
                    }
                }
                None => None,
            })
        }
        Ok(out)
    }

    fn get_transactions(
        &self,
        tx_ids: &[sovereign_sdk::rpc::TxIdentifier],
        _query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::TxResponse>>, anyhow::Error> {
        // TODO: https://github.com/Sovereign-Labs/sovereign/issues/191 Sort the input
        //      and use an iterator instead of querying for each slot individually
        let mut out = Vec::with_capacity(tx_ids.len());
        for id in tx_ids {
            let num = self.resolve_tx_identifier(id)?;
            out.push(match num {
                Some(num) => self.db.get::<TxByNumber>(&num)?.map(|tx| tx.into()),
                None => None,
            })
        }
        Ok(out)
    }

    fn get_events(
        &self,
        event_ids: &[sovereign_sdk::rpc::EventIdentifier],
    ) -> Result<Vec<Option<Self::EventResponse>>, anyhow::Error> {
        // TODO: Sort the input and use an iterator instead of querying for each slot individually
        // https://github.com/Sovereign-Labs/sovereign/issues/191
        let mut out = Vec::with_capacity(event_ids.len());
        for id in event_ids {
            let num = self.resolve_event_identifier(id)?;
            out.push(match num {
                Some(num) => self.db.get::<EventByNumber>(&num)?,
                None => None,
            })
        }
        Ok(out)
    }

    fn get_head(&self) -> Result<Option<Self::SlotResponse>, anyhow::Error> {
        let next_ids = self.get_next_items_numbers();
        let next_slot = next_ids.slot_number;

        let head_number = next_slot.saturating_sub(1);

        if let Some(stored_slot) = self
            .db
            .get::<SlotByNumber>(&SlotNumber(next_slot.saturating_sub(1)))?
        {
            return Ok(Some(self.populate_slot_response(
                head_number,
                stored_slot,
                QueryMode::Compact,
            )?));
        }
        Ok(None)
    }

    // Get X by hash
    fn get_slot_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::SlotResponse>, anyhow::Error> {
        self.get_slots(&[SlotIdentifier::Hash(*hash)], query_mode)
            .map(|mut batches: Vec<Option<SlotResponse>>| batches.pop().unwrap_or(None))
    }

    fn get_batch_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::BatchResponse>, anyhow::Error> {
        self.get_batches(&[BatchIdentifier::Hash(*hash)], query_mode)
            .map(|mut batches: Vec<Option<BatchResponse>>| batches.pop().unwrap_or(None))
    }

    fn get_tx_by_hash(
        &self,
        hash: &[u8; 32],
        query_mode: QueryMode,
    ) -> Result<Option<Self::TxResponse>, anyhow::Error> {
        self.get_transactions(&[TxIdentifier::Hash(*hash)], query_mode)
            .map(|mut txs: Vec<Option<TxResponse>>| txs.pop().unwrap_or(None))
    }

    // Get X by number
    fn get_slot_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::SlotResponse>, anyhow::Error> {
        self.get_slots(&[SlotIdentifier::Number(number)], query_mode)
            .map(|mut slots: Vec<Option<SlotResponse>>| slots.pop().unwrap_or(None))
    }

    fn get_batch_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::BatchResponse>, anyhow::Error> {
        self.get_batches(&[BatchIdentifier::Number(number)], query_mode)
            .map(|mut slots| slots.pop().unwrap_or(None))
    }

    fn get_tx_by_number(
        &self,
        number: u64,
        query_mode: QueryMode,
    ) -> Result<Option<Self::TxResponse>, anyhow::Error> {
        self.get_transactions(&[TxIdentifier::Number(number)], query_mode)
            .map(|mut txs| txs.pop().unwrap_or(None))
    }

    fn get_event_by_number(
        &self,
        number: u64,
    ) -> Result<Option<Self::EventResponse>, anyhow::Error> {
        self.get_events(&[EventIdentifier::Number(number)])
            .map(|mut events| events.pop().unwrap_or(None))
    }

    fn get_slots_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::SlotResponse>>, anyhow::Error> {
        let ids: Vec<_> = (start..=end).map(|n| SlotIdentifier::Number(n)).collect();
        self.get_slots(&ids, query_mode)
    }

    fn get_batches_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::BatchResponse>>, anyhow::Error> {
        let ids: Vec<_> = (start..=end)
            .map(|n| sovereign_sdk::rpc::BatchIdentifier::Number(n))
            .collect();
        self.get_batches(&ids, query_mode)
    }

    fn get_transactions_range(
        &self,
        start: u64,
        end: u64,
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::TxResponse>>, anyhow::Error> {
        let ids: Vec<_> = (start..=end)
            .map(|n| sovereign_sdk::rpc::TxIdentifier::Number(n))
            .collect();
        self.get_transactions(&ids, query_mode)
    }
}

impl LedgerDB {
    fn resolve_slot_identifier(
        &self,
        slot_id: &SlotIdentifier,
    ) -> Result<Option<SlotNumber>, anyhow::Error> {
        match slot_id {
            SlotIdentifier::Hash(hash) => self.db.get::<SlotByHash>(hash),
            SlotIdentifier::Number(num) => Ok(Some(SlotNumber(*num))),
        }
    }

    fn resolve_batch_identifier(
        &self,
        batch_id: &BatchIdentifier,
    ) -> Result<Option<BatchNumber>, anyhow::Error> {
        match batch_id {
            BatchIdentifier::Hash(hash) => self.db.get::<BatchByHash>(hash),
            BatchIdentifier::Number(num) => Ok(Some(BatchNumber(*num))),
            BatchIdentifier::SlotIdAndIndex(_) => todo!(),
        }
    }

    fn resolve_tx_identifier(
        &self,
        tx_id: &TxIdentifier,
    ) -> Result<Option<TxNumber>, anyhow::Error> {
        match tx_id {
            TxIdentifier::Hash(hash) => self.db.get::<TxByHash>(hash),
            TxIdentifier::Number(num) => Ok(Some(TxNumber(*num))),
            TxIdentifier::BatchIdAndIndex(_) => todo!(),
        }
    }

    fn resolve_event_identifier(
        &self,
        event_id: &EventIdentifier,
    ) -> Result<Option<EventNumber>, anyhow::Error> {
        match event_id {
            EventIdentifier::TxIdAndIndex((tx_id, offset)) => {
                if let Some(tx_num) = self.resolve_tx_identifier(tx_id)? {
                    Ok(self
                        .db
                        .get::<TxByNumber>(&tx_num)?
                        .map(|tx| EventNumber(tx.events.start.0 + offset)))
                } else {
                    Ok(None)
                }
            }
            EventIdentifier::Number(num) => Ok(Some(EventNumber(*num))),
            EventIdentifier::TxIdAndKey(_) => todo!(),
        }
    }

    fn populate_slot_response(
        &self,
        number: u64,
        slot: StoredSlot,
        mode: QueryMode,
    ) -> Result<SlotResponse, anyhow::Error> {
        Ok(match mode {
            QueryMode::Compact => SlotResponse {
                number,
                hash: slot.hash,
                batch_range: slot.batches,
                batches: None,
            },
            QueryMode::Standard => {
                let batches = self.get_batch_range(&slot.batches)?;
                let batch_hashes = Some(
                    batches
                        .into_iter()
                        .map(|batch| ItemOrHash::Hash(batch.hash))
                        .collect(),
                );
                SlotResponse {
                    number,
                    hash: slot.hash,
                    batch_range: slot.batches,
                    batches: batch_hashes,
                }
            }
            QueryMode::Full => {
                let num_batches = (slot.batches.end.0 - slot.batches.start.0) as usize;
                let mut batches = Vec::with_capacity(num_batches);
                for batch in self.get_batch_range(&slot.batches)? {
                    batches.push(ItemOrHash::Full(self.populate_batch_response(batch, mode)?));
                }

                SlotResponse {
                    number,
                    hash: slot.hash,
                    batch_range: slot.batches,
                    batches: Some(batches),
                }
            }
        })
    }

    fn populate_batch_response(
        &self,
        batch: StoredBatch,
        mode: QueryMode,
    ) -> Result<BatchResponse, anyhow::Error> {
        Ok(match mode {
            QueryMode::Compact => BatchResponse {
                hash: batch.hash,
                tx_range: batch.txs,
                txs: None,
                status: batch.status,
            },

            QueryMode::Standard => {
                let txs = self.get_tx_range(&batch.txs)?;
                let tx_hashes = Some(
                    txs.into_iter()
                        .map(|tx| ItemOrHash::Hash(tx.hash))
                        .collect(),
                );
                BatchResponse {
                    hash: batch.hash,
                    tx_range: batch.txs,
                    txs: tx_hashes,
                    status: batch.status,
                }
            }
            QueryMode::Full => {
                let num_txs = (batch.txs.end.0 - batch.txs.start.0) as usize;
                let mut txs = Vec::with_capacity(num_txs);
                for tx in self.get_tx_range(&batch.txs)? {
                    txs.push(ItemOrHash::Full(tx.into()));
                }

                BatchResponse {
                    hash: batch.hash,
                    tx_range: batch.txs,
                    txs: Some(txs),
                    status: batch.status,
                }
            }
        })
    }
}
