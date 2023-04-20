use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sovereign_sdk::{
    rpc::{LedgerRpcProvider, QueryMode, SlotIdentifier},
    spec::RollupSpec,
    stf::Event,
};

use crate::schema::{
    tables::{SlotByHash, SlotByNumber},
    types::{BatchNumber, EventNumber, SlotNumber, Status, StoredBatch, StoredSlot, TxNumber},
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

impl<S: RollupSpec> LedgerRpcProvider for LedgerDB<S> {
    type SlotResponse = SlotResponse;

    type BatchResponse = BatchResponse;

    type TxResponse = TxResponse;

    type EventResponse = Event;

    fn get_slots(
        &self,
        slot_ids: &[sovereign_sdk::rpc::SlotIdentifier],
        query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::SlotResponse>>, anyhow::Error> {
        // TODO: Sort the input and use an iterator instead of querying for each slot individually
        let mut out = Vec::with_capacity(slot_ids.len());
        for slot_id in slot_ids {
            let slot_num = self.resolve_slot_identifier(slot_id)?;
            out.push(match slot_num {
                Some(num) => {
                    if let Some(stored_slot) = self.db.get::<SlotByNumber>(&num)? {
                        Some(self.populate_slot_response(stored_slot, query_mode)?)
                    } else {
                        None
                    }
                }
                None => None,
            })
        }
        Ok(out)
    }

    fn get_batches(
        &self,
        _batch_ids: &[sovereign_sdk::rpc::BatchIdentifier],
        _query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::BatchResponse>>, anyhow::Error> {
        todo!()
    }

    fn get_transactions(
        &self,
        _tx_ids: &[sovereign_sdk::rpc::TxIdentifier],
        _query_mode: QueryMode,
    ) -> Result<Vec<Option<Self::TxResponse>>, anyhow::Error> {
        todo!()
    }

    fn get_events(
        &self,
        _event_ids: &[sovereign_sdk::rpc::EventIdentifier],
    ) -> Result<Option<Vec<Self::EventResponse>>, anyhow::Error> {
        todo!()
    }
}

impl<S: RollupSpec> LedgerDB<S> {
    fn resolve_slot_identifier(
        &self,
        slot_id: &SlotIdentifier,
    ) -> Result<Option<SlotNumber>, anyhow::Error> {
        match slot_id {
            SlotIdentifier::Hash(hash) => self.db.get::<SlotByHash>(hash),
            SlotIdentifier::Number(num) => Ok(Some(SlotNumber(*num))),
        }
    }

    fn populate_slot_response(
        &self,
        slot: StoredSlot,
        mode: QueryMode,
    ) -> Result<SlotResponse, anyhow::Error> {
        Ok(match mode {
            QueryMode::Compact => SlotResponse {
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
                    let response_tx = TxResponse {
                        hash: tx.hash,
                        event_range: tx.events,
                        body: Some(tx.data.as_ref().to_vec()),
                        status: tx.status,
                    };
                    txs.push(ItemOrHash::Full(response_tx));
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
