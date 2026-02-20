use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::sync::OnceLock;

use alloy_consensus::transaction::{Recovered, SignerRecoverable};
use alloy_consensus::{
    Block as ConsensusBlock, Header as ConsensusHeader, Receipt as ConsensusReceipt,
    ReceiptEnvelope, ReceiptWithBloom, Transaction as _, TxReceipt,
};
use alloy_eips::{BlockId, Encodable2718};
use alloy_primitives::private::alloy_rlp::{Decodable, Encodable as _, Header as RlpHeader};
use alloy_primitives::{Address, Bytes, Sealable, B256, U256, U64};
use alloy_rpc_types::eth::{Filter, FilterBlockOption};
use alloy_rpc_types::{
    AccessListResult, Block, BlockNumberOrTag, BlockTransactions, FeeHistory, Header, Log,
    TransactionReceipt, TransactionRequest,
};
use jsonrpsee::types::ErrorObjectOwned;
use revm::context::result::{ExecutionResult, ResultAndState};
use revm::context::{BlockEnv, CfgEnv};
use revm::database::CacheDB;
use revm::database_interface::EmptyDB;
use revm::primitives::hardfork::SpecId;
use revm::state::{AccountInfo, Bytecode};
use revm::DatabaseCommit;
use revm_inspectors::access_list::AccessListInspector;
use serde_json::Value;
use sov_rpc_eth_types::{
    invalid_params_rpc_err, rpc_error_with_code, EthApiError, LogWithExecutionTimestamp,
};
use tracing::warn;

use crate::evm::conversions::create_block_env;
use crate::evm::primitive_types::TransactionSigned;
use crate::helpers::{from_recovered_with_block_context, prepare_call_env};
use crate::rpc::{
    with_block_timestamp, with_block_transaction_timestamps, BlockWithTransactionTimestamp,
    TransactionWithBlockTimestamp,
};
use crate::Receipt;

const CHAIN_RLP_ENV: &str = "SOV_HIVE_CHAIN_RLP_PATH";
const GENESIS_JSON_ENV: &str = "SOV_HIVE_GENESIS_JSON_PATH";

static IMPORTED_CHAIN: OnceLock<Option<ImportedChain>> = OnceLock::new();

pub(crate) fn latest_block_number() -> Option<u64> {
    imported_chain().map(|c| c.latest_block_number)
}

pub(crate) fn block_by_id(block_id: BlockId, full: bool) -> Option<BlockWithTransactionTimestamp> {
    let chain = imported_chain()?;
    let block = chain.block_by_id(block_id)?;
    block.to_rpc(full)
}

pub(crate) fn block_by_hash(block_hash: B256, full: bool) -> Option<BlockWithTransactionTimestamp> {
    let chain = imported_chain()?;
    let block = chain.block_by_hash(&block_hash)?;
    block.to_rpc(full)
}

pub(crate) fn tx_by_hash(hash: B256) -> Option<TransactionWithBlockTimestamp> {
    let chain = imported_chain()?;
    chain.tx_by_hash(&hash)
}

pub(crate) fn tx_by_block_hash_and_index(
    block_hash: B256,
    index: u64,
) -> Option<TransactionWithBlockTimestamp> {
    let chain = imported_chain()?;
    let block = chain.block_by_hash(&block_hash)?;
    block.tx_by_index(index)
}

pub(crate) fn tx_by_block_number_and_index(
    block: BlockNumberOrTag,
    index: u64,
) -> Option<TransactionWithBlockTimestamp> {
    let chain = imported_chain()?;
    let block = chain.block_by_number_tag(block)?;
    block.tx_by_index(index)
}

pub(crate) fn block_tx_count_by_hash(block_hash: B256) -> Option<U64> {
    let chain = imported_chain()?;
    if Some(block_hash) == chain.inferred_genesis_hash {
        return Some(U64::ZERO);
    }
    Some(U64::from(
        chain.block_by_hash(&block_hash)?.transactions.len() as u64,
    ))
}

pub(crate) fn block_tx_count_by_id(block_id: BlockId) -> Option<U64> {
    let chain = imported_chain()?;
    Some(U64::from(
        chain.block_by_id(block_id)?.transactions.len() as u64
    ))
}

pub(crate) fn raw_block(block_id: BlockId) -> Option<Bytes> {
    let chain = imported_chain()?;
    Some(chain.block_by_id(block_id)?.raw_block.clone().into())
}

pub(crate) fn raw_header(block_id: BlockId) -> Option<Bytes> {
    let chain = imported_chain()?;
    Some(chain.block_by_id(block_id)?.raw_header.clone().into())
}

pub(crate) fn raw_tx(hash: B256) -> Option<Bytes> {
    let chain = imported_chain()?;
    let tx = chain.tx_by_hash.get(&hash)?;
    let block = chain.blocks_by_number.get(&tx.block_number)?;
    let tx = block.transactions.get(tx.tx_index)?;
    Some(tx.encoded_2718().into())
}

pub(crate) fn transaction_receipt(
    hash: B256,
) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
    let chain = imported_chain()?;
    let tx_ref = chain.tx_by_hash.get(&hash)?;
    let block = chain.blocks_by_number.get(&tx_ref.block_number)?;
    let replay_state = chain.replay_state.as_ref()?;
    let receipts = replay_state.receipts_by_block.get(&block.number)?;
    let receipt = receipts.get(tx_ref.tx_index)?;
    let tx = block.transactions.get(tx_ref.tx_index)?;
    to_rpc_receipt(block, tx, receipt)
}

pub(crate) fn block_receipts(
    block_id: BlockId,
) -> Option<Vec<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>>> {
    let chain = imported_chain()?;
    let block = chain.block_by_id(block_id)?;
    let replay_state = chain.replay_state.as_ref()?;
    let receipts = replay_state.receipts_by_block.get(&block.number)?;
    if receipts.len() != block.transactions.len() {
        return None;
    }

    let mut out = Vec::with_capacity(receipts.len());
    for (tx, receipt) in block.transactions.iter().zip(receipts) {
        out.push(to_rpc_receipt(block, tx, receipt)?);
    }
    Some(out)
}

pub(crate) fn raw_receipts(block_id: BlockId) -> Option<Vec<Bytes>> {
    let chain = imported_chain()?;
    let block = chain.block_by_id(block_id)?;
    let replay_state = chain.replay_state.as_ref()?;
    replay_state
        .raw_receipts_by_block
        .get(&block.number)
        .cloned()
}

pub(crate) fn get_logs(
    filter: &Filter,
) -> Option<Result<Vec<LogWithExecutionTimestamp>, ErrorObjectOwned>> {
    let chain = imported_chain()?;
    let replay_state = chain.replay_state.as_ref()?;

    if filter.block_option.ensure_valid_block_range().is_err() {
        return Some(Err(invalid_params_rpc_err("invalid block range params")));
    }

    let mut logs = Vec::new();
    match filter.block_option {
        FilterBlockOption::AtBlockHash(block_hash) => {
            let Some(block) = chain.block_by_hash(&block_hash) else {
                return Some(Err(rpc_error_with_code(
                    -32001,
                    format!("Block with hash {block_hash} not found"),
                )));
            };
            append_block_logs(filter, block, replay_state, &mut logs);
        }
        FilterBlockOption::Range {
            from_block,
            to_block,
        } => {
            let latest = chain.latest_block_number;
            let from = chain.block_number_from_tag(from_block.unwrap_or(BlockNumberOrTag::Latest));
            let to = chain.block_number_from_tag(to_block.unwrap_or(BlockNumberOrTag::Latest));
            if from > to || to > latest {
                return Some(Err(invalid_params_rpc_err("invalid block range params")));
            }

            for block_number in from..=to {
                let Some(block) = chain.blocks_by_number.get(&block_number) else {
                    continue;
                };
                append_block_logs(filter, block, replay_state, &mut logs);
            }
        }
    }

    Some(Ok(logs))
}

pub(crate) fn get_balance(address: Address, block_id: BlockId) -> Option<U256> {
    let chain = imported_chain()?;
    let (block, replay) = chain.replay_snapshot_by_id(block_id)?;
    let _ = block;

    let balance = replay
        .cache
        .accounts
        .get(&address)
        .map(|account| account.info.balance)
        .unwrap_or_default();
    Some(balance)
}

pub(crate) fn get_storage(address: Address, slot: U256, block_id: BlockId) -> Option<B256> {
    let chain = imported_chain()?;
    let (block, replay) = chain.replay_snapshot_by_id(block_id)?;
    let _ = block;

    let value = replay
        .cache
        .accounts
        .get(&address)
        .and_then(|account| account.storage.get(&slot).copied())
        .unwrap_or_default();
    Some(value.to_be_bytes::<32>().into())
}

pub(crate) fn get_transaction_count(address: Address, block_id: BlockId) -> Option<U64> {
    let chain = imported_chain()?;
    let (block, replay) = chain.replay_snapshot_by_id(block_id)?;
    let _ = block;

    let nonce = replay
        .cache
        .accounts
        .get(&address)
        .map(|account| account.info.nonce)
        .unwrap_or_default();
    Some(U64::from(nonce))
}

pub(crate) fn get_code(address: Address, block_id: BlockId) -> Option<Bytes> {
    let chain = imported_chain()?;
    let (block, replay) = chain.replay_snapshot_by_id(block_id)?;
    let _ = block;

    let code = replay
        .cache
        .accounts
        .get(&address)
        .and_then(|account| replay.cache.contracts.get(&account.info.code_hash))
        .map(|code| Bytes::copy_from_slice(code.original_byte_slice()))
        .unwrap_or_default();
    Some(code)
}

pub(crate) fn call(
    request: TransactionRequest,
    block_id: Option<BlockId>,
) -> Option<Result<ResultAndState, EthApiError>> {
    let chain = imported_chain()?;
    let (block, replay_state, replay_db) =
        chain.replay_context_for_call(block_id.unwrap_or_else(BlockId::latest))?;
    let mut db = replay_db.clone();

    let use_zero_gas_price = request.gas_price.is_none()
        && request.max_fee_per_gas.is_none()
        && request.max_priority_fee_per_gas.is_none();

    let block_env = block_env_from_header(&block.header);
    let mut tx_env = match prepare_call_env(&block_env, request) {
        Ok(tx_env) => tx_env,
        Err(err) => return Some(Err(err)),
    };
    if use_zero_gas_price {
        tx_env.gas_price = 0;
    }

    let cfg = replay_cfg_env(
        replay_state.chain_id,
        replay_state
            .forks
            .spec_id_for_block(block.number, block.header.timestamp),
    );

    Some(crate::executor::transact(&mut db, &block_env, tx_env, cfg).map_err(EthApiError::from))
}

pub(crate) fn create_access_list(
    request: TransactionRequest,
    block_id: Option<BlockId>,
) -> Option<Result<AccessListResult, EthApiError>> {
    let chain = imported_chain()?;
    let (block, replay_state, replay_db) =
        chain.replay_context_for_call(block_id.unwrap_or_else(BlockId::latest))?;
    let db = replay_db.clone();

    let initial_access_list = request.access_list.clone().unwrap_or_default();
    let block_env = block_env_from_header(&block.header);
    let tx_env = match prepare_call_env(&block_env, request) {
        Ok(tx_env) => tx_env,
        Err(err) => return Some(Err(err)),
    };

    let cfg = replay_cfg_env(
        replay_state.chain_id,
        replay_state
            .forks
            .spec_id_for_block(block.number, block.header.timestamp),
    );

    let mut inspector = AccessListInspector::new(initial_access_list);
    let execution = crate::executor::inspect(db, &block_env, tx_env, cfg, &mut inspector)
        .map_err(EthApiError::from);

    Some(execution.map(|execution| {
        let (gas_used, error) = match execution.result {
            ExecutionResult::Success { gas_used, .. } => (U256::from(gas_used), None),
            ExecutionResult::Revert { gas_used, .. } => {
                (U256::from(gas_used), Some("execution reverted".to_string()))
            }
            ExecutionResult::Halt { gas_used, reason } => {
                (U256::from(gas_used), Some(format!("{reason:?}")))
            }
        };

        AccessListResult {
            access_list: inspector.into_access_list(),
            gas_used,
            error,
        }
    }))
}

pub(crate) fn fee_history(
    block_count: u64,
    newest_block: BlockNumberOrTag,
    reward_percentiles: Option<&[f64]>,
) -> Option<FeeHistory> {
    if block_count == 0 {
        return None;
    }

    let chain = imported_chain()?;
    let end = chain.resolve_block_number_tag(newest_block)?;
    let start = end.saturating_sub(block_count.saturating_sub(1));

    let mut base_fees = Vec::new();
    let mut gas_used_ratio = Vec::new();

    #[allow(clippy::float_arithmetic)]
    for n in start..=end {
        let block = chain.blocks_by_number.get(&n)?;
        let base_fee = block.header.base_fee_per_gas.unwrap_or_default();
        base_fees.push(u128::from(base_fee));

        let gas_limit = block.header.gas_limit.max(1);
        gas_used_ratio.push(block.header.gas_used as f64 / gas_limit as f64);
    }

    let next_fee = base_fees.last().copied().unwrap_or_default();
    base_fees.push(next_fee);

    let reward = reward_percentiles
        .filter(|percentiles| !percentiles.is_empty())
        .map(|percentiles| {
            vec![vec![0u128; percentiles.len()]; (end.saturating_sub(start) + 1) as usize]
        });

    Some(FeeHistory {
        oldest_block: start,
        base_fee_per_gas: base_fees,
        gas_used_ratio,
        reward,
        blob_gas_used_ratio: vec![0.0; (end.saturating_sub(start) + 1) as usize],
        base_fee_per_blob_gas: vec![0; (end.saturating_sub(start) + 2) as usize],
    })
}

fn imported_chain() -> Option<&'static ImportedChain> {
    IMPORTED_CHAIN.get_or_init(ImportedChain::load).as_ref()
}

#[derive(Clone)]
struct ImportedBlock {
    number: u64,
    hash: B256,
    header: ConsensusHeader,
    transactions: Vec<TransactionSigned>,
    uncles: Vec<B256>,
    withdrawals: Option<alloy_eips::eip4895::Withdrawals>,
    raw_block: Vec<u8>,
    raw_header: Vec<u8>,
}

impl ImportedBlock {
    fn to_rpc(&self, full: bool) -> Option<BlockWithTransactionTimestamp> {
        let transactions = if full {
            let mut txs = Vec::with_capacity(self.transactions.len());
            for (idx, tx) in self.transactions.iter().cloned().enumerate() {
                let signer = tx.recover_signer().ok()?;
                let recovered = Recovered::new_unchecked(tx, signer);
                txs.push(from_recovered_with_block_context(
                    recovered,
                    Some(self.hash),
                    self.number,
                    idx as u64,
                    self.header.base_fee_per_gas,
                ));
            }
            BlockTransactions::Full(txs)
        } else {
            BlockTransactions::Hashes(self.transactions.iter().map(|tx| *tx.hash()).collect())
        };

        let header = Header::from_consensus(
            self.header.clone().seal_slow(),
            None,
            Some(U256::from(self.raw_block.len())),
        );
        Some(with_block_transaction_timestamps(Block {
            header,
            uncles: self.uncles.clone(),
            transactions,
            withdrawals: self.withdrawals.clone(),
        }))
    }

    fn tx_by_index(&self, index: u64) -> Option<TransactionWithBlockTimestamp> {
        let tx = self.transactions.get(index as usize)?.clone();
        let signer = tx.recover_signer().ok()?;
        let recovered = Recovered::new_unchecked(tx, signer);
        Some(with_block_timestamp(
            from_recovered_with_block_context(
                recovered,
                Some(self.hash),
                self.number,
                index,
                self.header.base_fee_per_gas,
            ),
            Some(self.header.timestamp),
        ))
    }
}

#[derive(Clone, Copy)]
struct ImportedTxRef {
    block_number: u64,
    tx_index: usize,
}

struct ImportedChain {
    blocks_by_number: BTreeMap<u64, ImportedBlock>,
    blocks_by_hash: HashMap<B256, u64>,
    tx_by_hash: HashMap<B256, ImportedTxRef>,
    inferred_genesis_hash: Option<B256>,
    replay_state: Option<ReplayState>,
    latest_block_number: u64,
}

impl ImportedChain {
    fn load() -> Option<Self> {
        let chain_path = std::env::var(CHAIN_RLP_ENV).ok()?;
        let data = fs::read(&chain_path).ok()?;
        if data.is_empty() {
            return None;
        }

        let mut blocks_by_number = BTreeMap::new();
        let mut blocks_by_hash = HashMap::new();
        let mut tx_by_hash = HashMap::new();
        let mut inferred_genesis_hash = None;

        let mut offset = 0usize;
        while offset < data.len() {
            let mut header_input = &data[offset..];
            let rlp_header = RlpHeader::decode(&mut header_input).ok()?;
            if !rlp_header.list {
                warn!("Ignoring Hive chain fallback: top-level item is not an RLP list");
                return None;
            }

            let prefix_len = data[offset..].len().saturating_sub(header_input.len());
            let total_len = prefix_len.checked_add(rlp_header.payload_length)?;
            let end = offset.checked_add(total_len)?;
            if end > data.len() {
                warn!("Ignoring Hive chain fallback: malformed /chain.rlp");
                return None;
            }

            let raw_block = data[offset..end].to_vec();
            let mut decode_input = raw_block.as_slice();
            let decoded: ConsensusBlock<TransactionSigned, ConsensusHeader> =
                <ConsensusBlock<TransactionSigned, ConsensusHeader> as Decodable>::decode(
                    &mut decode_input,
                )
                .ok()?;
            if !decode_input.is_empty() {
                warn!("Ignoring Hive chain fallback: malformed block payload");
                return None;
            }

            let mut raw_header = Vec::new();
            decoded.header.encode(&mut raw_header);

            let block_hash = decoded.header.hash_slow();
            let block_number = decoded.header.number;
            if inferred_genesis_hash.is_none() {
                inferred_genesis_hash = Some(decoded.header.parent_hash);
            }
            let uncles = decoded
                .body
                .ommers
                .iter()
                .map(ConsensusHeader::hash_slow)
                .collect::<Vec<_>>();

            let transactions = decoded.body.transactions;
            for (idx, tx) in transactions.iter().enumerate() {
                tx_by_hash.insert(
                    *tx.hash(),
                    ImportedTxRef {
                        block_number,
                        tx_index: idx,
                    },
                );
            }

            let imported = ImportedBlock {
                number: block_number,
                hash: block_hash,
                header: decoded.header,
                transactions,
                uncles,
                withdrawals: decoded.body.withdrawals,
                raw_block,
                raw_header,
            };
            blocks_by_hash.insert(block_hash, block_number);
            blocks_by_number.insert(block_number, imported);
            offset = end;
        }

        let latest_block_number = *blocks_by_number.keys().max()?;
        let replay_state = ReplayState::build(&blocks_by_number);

        Some(Self {
            blocks_by_number,
            blocks_by_hash,
            tx_by_hash,
            inferred_genesis_hash,
            replay_state,
            latest_block_number,
        })
    }

    fn block_by_hash(&self, hash: &B256) -> Option<&ImportedBlock> {
        let number = self.blocks_by_hash.get(hash)?;
        self.blocks_by_number.get(number)
    }

    fn block_by_number_tag(&self, tag: BlockNumberOrTag) -> Option<&ImportedBlock> {
        match tag {
            BlockNumberOrTag::Earliest => self
                .blocks_by_number
                .get(&self.blocks_by_number.keys().next().copied()?),
            BlockNumberOrTag::Latest
            | BlockNumberOrTag::Safe
            | BlockNumberOrTag::Finalized
            | BlockNumberOrTag::Pending => self.blocks_by_number.get(&self.latest_block_number),
            BlockNumberOrTag::Number(number) => self.blocks_by_number.get(&number),
        }
    }

    fn resolve_block_number_tag(&self, tag: BlockNumberOrTag) -> Option<u64> {
        self.block_by_number_tag(tag).map(|block| block.number)
    }

    fn block_number_from_tag(&self, tag: BlockNumberOrTag) -> u64 {
        match tag {
            BlockNumberOrTag::Earliest => self.blocks_by_number.keys().next().copied().unwrap_or(0),
            BlockNumberOrTag::Latest
            | BlockNumberOrTag::Safe
            | BlockNumberOrTag::Finalized
            | BlockNumberOrTag::Pending => self.latest_block_number,
            BlockNumberOrTag::Number(number) => number,
        }
    }

    fn block_by_id(&self, block_id: BlockId) -> Option<&ImportedBlock> {
        match block_id {
            BlockId::Hash(hash) => self.block_by_hash(&hash.block_hash),
            BlockId::Number(tag) => self.block_by_number_tag(tag),
        }
    }

    fn replay_snapshot_by_id(
        &self,
        block_id: BlockId,
    ) -> Option<(&ImportedBlock, &CacheDB<EmptyDB>)> {
        let block = self.block_by_id(block_id)?;
        let replay_state = self.replay_state.as_ref()?;
        let snapshot = replay_state.db_by_block.get(&block.number)?;
        Some((block, snapshot))
    }

    fn replay_context_for_call(
        &self,
        block_id: BlockId,
    ) -> Option<(&ImportedBlock, &ReplayState, &CacheDB<EmptyDB>)> {
        let block = self.block_by_id(block_id)?;
        let replay_state = self.replay_state.as_ref()?;
        let snapshot = replay_state.db_by_block.get(&block.number)?;
        Some((block, replay_state, snapshot))
    }

    fn tx_by_hash(&self, hash: &B256) -> Option<TransactionWithBlockTimestamp> {
        let tx_ref = self.tx_by_hash.get(hash)?;
        let block = self.blocks_by_number.get(&tx_ref.block_number)?;
        block.tx_by_index(tx_ref.tx_index as u64)
    }
}

#[derive(Clone, Copy)]
struct ForkConfig {
    chain_id: u64,
    homestead_block: u64,
    eip150_block: u64,
    eip158_block: u64,
    byzantium_block: u64,
    petersburg_block: u64,
    istanbul_block: u64,
    muir_glacier_block: u64,
    berlin_block: u64,
    london_block: u64,
    merge_netsplit_block: u64,
    shanghai_time: Option<u64>,
    cancun_time: Option<u64>,
    prague_time: Option<u64>,
}

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
            chain_id: 0,
            homestead_block: 0,
            eip150_block: 0,
            eip158_block: 0,
            byzantium_block: 0,
            petersburg_block: 0,
            istanbul_block: 0,
            muir_glacier_block: 0,
            berlin_block: 0,
            london_block: 0,
            merge_netsplit_block: 0,
            shanghai_time: None,
            cancun_time: None,
            prague_time: None,
        }
    }
}

impl ForkConfig {
    fn from_genesis(genesis: &Value) -> Self {
        let config = genesis.get("config").and_then(Value::as_object);

        let get_block = |name: &str| -> u64 {
            config
                .and_then(|cfg| cfg.get(name))
                .and_then(parse_json_u64)
                .unwrap_or_default()
        };

        let get_time = |name: &str| -> Option<u64> {
            config
                .and_then(|cfg| cfg.get(name))
                .and_then(parse_json_u64)
        };

        Self {
            chain_id: config
                .and_then(|cfg| cfg.get("chainId"))
                .and_then(parse_json_u64)
                .unwrap_or_default(),
            homestead_block: get_block("homesteadBlock"),
            eip150_block: get_block("eip150Block"),
            eip158_block: get_block("eip158Block"),
            byzantium_block: get_block("byzantiumBlock"),
            petersburg_block: get_block("petersburgBlock"),
            istanbul_block: get_block("istanbulBlock"),
            muir_glacier_block: get_block("muirGlacierBlock"),
            berlin_block: get_block("berlinBlock"),
            london_block: get_block("londonBlock"),
            merge_netsplit_block: get_block("mergeNetsplitBlock"),
            shanghai_time: get_time("shanghaiTime"),
            cancun_time: get_time("cancunTime"),
            prague_time: get_time("pragueTime"),
        }
    }

    fn spec_id_for_block(&self, number: u64, timestamp: u64) -> SpecId {
        let mut spec = SpecId::FRONTIER;

        if number >= self.homestead_block {
            spec = SpecId::HOMESTEAD;
        }
        if number >= self.eip150_block {
            spec = SpecId::TANGERINE;
        }
        if number >= self.eip158_block {
            spec = SpecId::SPURIOUS_DRAGON;
        }
        if number >= self.byzantium_block {
            spec = SpecId::BYZANTIUM;
        }
        if number >= self.petersburg_block {
            spec = SpecId::PETERSBURG;
        }
        if number >= self.istanbul_block {
            spec = SpecId::ISTANBUL;
        }
        if number >= self.muir_glacier_block {
            spec = SpecId::MUIR_GLACIER;
        }
        if number >= self.berlin_block {
            spec = SpecId::BERLIN;
        }
        if number >= self.london_block {
            spec = SpecId::LONDON;
        }
        if number >= self.merge_netsplit_block {
            spec = SpecId::MERGE;
        }

        if let Some(shanghai_time) = self.shanghai_time {
            if timestamp >= shanghai_time {
                spec = SpecId::SHANGHAI;
            }
        }
        if let Some(cancun_time) = self.cancun_time {
            if timestamp >= cancun_time {
                spec = SpecId::CANCUN;
            }
        }
        if let Some(prague_time) = self.prague_time {
            if timestamp >= prague_time {
                spec = SpecId::PRAGUE;
            }
        }

        spec
    }
}

struct ReplayState {
    db_by_block: BTreeMap<u64, CacheDB<EmptyDB>>,
    receipts_by_block: BTreeMap<u64, Vec<Receipt>>,
    raw_receipts_by_block: BTreeMap<u64, Vec<Bytes>>,
    forks: ForkConfig,
    chain_id: u64,
}

impl ReplayState {
    fn build(blocks_by_number: &BTreeMap<u64, ImportedBlock>) -> Option<Self> {
        let genesis_path =
            std::env::var(GENESIS_JSON_ENV).unwrap_or_else(|_| "/genesis.json".to_string());
        let genesis_bytes = fs::read(&genesis_path).ok()?;
        let genesis_json: Value = serde_json::from_slice(&genesis_bytes).ok()?;
        let forks = ForkConfig::from_genesis(&genesis_json);
        let chain_id = forks.chain_id.max(1);

        let mut db = CacheDB::new(EmptyDB::default());
        seed_replay_db_from_genesis(&mut db, &genesis_json)?;

        let mut db_by_block = BTreeMap::new();
        let mut receipts_by_block = BTreeMap::new();
        let mut raw_receipts_by_block = BTreeMap::new();
        for block in blocks_by_number.values() {
            let block_env = block_env_from_header(&block.header);
            let cfg = replay_cfg_env(
                chain_id,
                forks.spec_id_for_block(block.number, block.header.timestamp),
            );

            let mut cumulative_gas_used = 0u64;
            let mut log_index_start = 0u64;
            let mut receipts = Vec::with_capacity(block.transactions.len());
            let mut raw_receipts = Vec::with_capacity(block.transactions.len());
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                let signer = tx.recover_signer().ok()?;
                let tx_env = crate::create_tx_env(tx, signer, tx.nonce(), tx.gas_limit());
                let ResultAndState { result, state } =
                    crate::executor::transact(&mut db, &block_env, tx_env, cfg.clone())
                    .map_err(|err| {
                        warn!(?err, number = block.number, hash = %tx.hash(), "Failed to replay imported tx");
                    })
                    .ok()?;
                db.commit(state);

                let is_success = result.is_success();
                let gas_used = result.gas_used();
                cumulative_gas_used = cumulative_gas_used.checked_add(gas_used)?;
                let logs = result.into_logs();

                let receipt = reth_primitives::Receipt {
                    tx_type: tx.tx_type(),
                    success: is_success,
                    cumulative_gas_used,
                    logs,
                };

                let evm_receipt = Receipt {
                    receipt: receipt.clone(),
                    transaction_hash: *tx.hash(),
                    transaction_index: tx_index as u64,
                    block_number: block.number,
                    gas_used,
                    log_index_start,
                };
                log_index_start =
                    log_index_start.checked_add(evm_receipt.receipt.logs.len() as u64)?;

                let logs_bloom = evm_receipt.receipt.bloom();
                let typed_receipt = ConsensusReceipt {
                    status: evm_receipt.receipt.success.into(),
                    cumulative_gas_used: evm_receipt.receipt.cumulative_gas_used,
                    logs: evm_receipt.receipt.logs.clone(),
                };
                let envelope = ReceiptEnvelope::from_typed(
                    tx.tx_type(),
                    ReceiptWithBloom::new(typed_receipt, logs_bloom),
                );
                raw_receipts.push(envelope.encoded_2718().into());
                receipts.push(evm_receipt);
            }

            db_by_block.insert(block.number, db.clone());
            receipts_by_block.insert(block.number, receipts);
            raw_receipts_by_block.insert(block.number, raw_receipts);
        }

        Some(Self {
            db_by_block,
            receipts_by_block,
            raw_receipts_by_block,
            forks,
            chain_id,
        })
    }
}

fn to_rpc_receipt(
    block: &ImportedBlock,
    tx: &TransactionSigned,
    receipt: &Receipt,
) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
    let signer = tx.recover_signer().ok()?;
    let recovered = Recovered::new_unchecked(tx.clone(), signer);
    let from = recovered.signer();

    let logs: Vec<LogWithExecutionTimestamp> = receipt
        .receipt
        .logs
        .iter()
        .cloned()
        .enumerate()
        .map(|(tx_log_idx, log)| LogWithExecutionTimestamp {
            log: Log {
                inner: log,
                block_hash: Some(block.hash),
                block_number: Some(block.number),
                block_timestamp: Some(block.header.timestamp),
                transaction_hash: Some(receipt.transaction_hash),
                transaction_index: Some(receipt.transaction_index),
                log_index: Some(receipt.log_index_start + tx_log_idx as u64),
                removed: false,
            },
            // Replay does not track real execution timestamps; use block timestamp in ms.
            time_executed_ms: block.header.timestamp.saturating_mul(1000),
        })
        .collect();

    let rpc_receipt = alloy_rpc_types::Receipt {
        status: receipt.receipt.success.into(),
        cumulative_gas_used: receipt.receipt.cumulative_gas_used,
        logs,
    };
    let logs_bloom = receipt.receipt.bloom();
    let inner =
        ReceiptEnvelope::from_typed(tx.tx_type(), ReceiptWithBloom::new(rpc_receipt, logs_bloom));

    let (contract_address, to) = match recovered.inner().kind() {
        alloy_primitives::TxKind::Create => (Some(from.create(recovered.nonce())), None),
        alloy_primitives::TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    Some(TransactionReceipt {
        inner,
        transaction_hash: receipt.transaction_hash,
        transaction_index: Some(receipt.transaction_index),
        block_hash: Some(block.hash),
        block_number: Some(block.number),
        gas_used: receipt.gas_used,
        effective_gas_price: recovered
            .inner()
            .effective_gas_price(block.header.base_fee_per_gas),
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
    })
}

fn append_block_logs(
    filter: &Filter,
    block: &ImportedBlock,
    replay_state: &ReplayState,
    out: &mut Vec<LogWithExecutionTimestamp>,
) {
    if !filter.matches_bloom(block.header.logs_bloom) {
        return;
    }
    let Some(receipts) = replay_state.receipts_by_block.get(&block.number) else {
        return;
    };

    for receipt in receipts {
        for (tx_log_idx, log) in receipt.receipt.logs.iter().cloned().enumerate() {
            if !filter.matches(&log) {
                continue;
            }
            out.push(LogWithExecutionTimestamp {
                log: Log {
                    inner: log,
                    block_hash: Some(block.hash),
                    block_number: Some(block.number),
                    block_timestamp: Some(block.header.timestamp),
                    transaction_hash: Some(receipt.transaction_hash),
                    transaction_index: Some(receipt.transaction_index),
                    log_index: Some(receipt.log_index_start + tx_log_idx as u64),
                    removed: false,
                },
                time_executed_ms: block.header.timestamp.saturating_mul(1000),
            });
        }
    }
}

fn seed_replay_db_from_genesis(db: &mut CacheDB<EmptyDB>, genesis: &Value) -> Option<()> {
    let alloc = genesis.get("alloc")?.as_object()?;

    for (raw_address, entry) in alloc {
        let address = parse_hex_address(raw_address)?;
        let balance = entry
            .get("balance")
            .and_then(Value::as_str)
            .and_then(parse_hex_u256)
            .unwrap_or_default();
        let nonce = entry
            .get("nonce")
            .and_then(Value::as_str)
            .and_then(parse_hex_u64)
            .unwrap_or_default();
        let code = entry
            .get("code")
            .and_then(Value::as_str)
            .and_then(parse_hex_bytes);

        let code_hash = code
            .as_ref()
            .map(|code| alloy_primitives::keccak256(code.as_ref()))
            .unwrap_or_default();
        let account_info = AccountInfo {
            balance,
            nonce,
            code_hash,
            code: code.map(Bytecode::new_raw),
        };
        db.insert_account_info(address, account_info);

        if let Some(storage) = entry.get("storage").and_then(Value::as_object) {
            for (raw_slot, raw_value) in storage {
                let slot = parse_hex_u256(raw_slot)?;
                let value = raw_value.as_str().and_then(parse_hex_u256)?;
                db.insert_account_storage(address, slot, value).ok()?;
            }
        }
    }

    Some(())
}

fn parse_json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(string) => parse_hex_or_decimal_u64(string),
        _ => None,
    }
}

fn parse_hex_or_decimal_u64(raw: &str) -> Option<u64> {
    if let Some(stripped) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(stripped, 16).ok()
    } else {
        raw.parse::<u64>().ok()
    }
}

fn parse_hex_u64(raw: &str) -> Option<u64> {
    parse_hex_or_decimal_u64(raw)
}

fn parse_hex_u256(raw: &str) -> Option<U256> {
    let stripped = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if stripped.is_empty() {
        return Some(U256::ZERO);
    }
    let normalized = if stripped.len() % 2 == 0 {
        stripped.to_string()
    } else {
        format!("0{stripped}")
    };
    let bytes = hex::decode(normalized).ok()?;
    Some(U256::from_be_slice(&bytes))
}

fn parse_hex_bytes(raw: &str) -> Option<Bytes> {
    let stripped = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if stripped.is_empty() {
        return Some(Bytes::default());
    }
    let normalized = if stripped.len() % 2 == 0 {
        stripped.to_string()
    } else {
        format!("0{stripped}")
    };
    Some(hex::decode(normalized).ok()?.into())
}

fn parse_hex_address(raw: &str) -> Option<Address> {
    let stripped = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if stripped.len() != 40 {
        return None;
    }
    let bytes = hex::decode(stripped).ok()?;
    Some(Address::from_slice(&bytes))
}

fn replay_cfg_env(chain_id: u64, spec: SpecId) -> CfgEnv<SpecId> {
    let mut cfg: CfgEnv<SpecId> = CfgEnv::default();
    cfg.disable_block_gas_limit = false;
    cfg.disable_eip3607 = true;
    cfg.disable_base_fee = true;
    cfg.tx_chain_id_check = false;
    cfg.chain_id = chain_id;
    cfg.limit_contract_code_size = None;
    cfg.memory_limit = 50 * 1024 * 1024;
    cfg.with_spec(spec)
}

fn block_env_from_header(header: &ConsensusHeader) -> BlockEnv {
    create_block_env(
        header.base_fee_per_gas.unwrap_or_default(),
        header.gas_limit,
        header.timestamp,
        header.beneficiary,
        header.number,
        Some(header.mix_hash),
    )
}
