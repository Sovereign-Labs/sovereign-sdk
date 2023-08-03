use std::sync::Arc;

use anyhow::Result;
use reth_interfaces::provider::ProviderError;
use reth_interfaces::Result as RethResult;
use reth_primitives::{
    Address, BlockNumber, Bytes, ChainSpec, ChainSpecBuilder, SealedBlock, SealedBlockWithSenders,
    SealedHeader, StorageValue, H256, U256,
};
use reth_provider::{
    AccountReader, BlockExecutor, BlockHashReader, PostState, StateProvider, StateRootProvider,
};
use reth_revm::database::{State, SubState};
use reth_revm::executor::Executor;
use revm::primitives::{Account, Bytecode, CfgEnv};
use sov_modules_api::{CallResponse, Context};
use sov_state::storage::StorageKey;
use sov_state::WorkingSet;

use crate::evm::contract_address;
use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::transaction::EvmTransaction;
use crate::{Evm, TransactionReceipt};

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    pub tx: Vec<EvmTransaction>,
}

struct InMemDb {}

impl StateRootProvider for InMemDb {
    fn state_root(&self, post_state: PostState) -> RethResult<H256> {
        todo!()
    }
}

impl AccountReader for InMemDb {
    fn basic_account(
        &self,
        address: Address,
    ) -> std::result::Result<std::option::Option<reth_primitives::Account>, reth_interfaces::Error>
    {
        todo!()
    }
}

impl BlockHashReader for InMemDb {
    fn block_hash(&self, number: BlockNumber) -> RethResult<Option<H256>> {
        todo!()
    }

    #[doc = " Get headers in range of block hashes or numbers"]
    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> RethResult<Vec<H256>> {
        todo!()
    }
}

impl StateProvider for InMemDb {
    fn storage(
        &self,
        account: Address,
        storage_key: reth_primitives::H256,
    ) -> RethResult<Option<StorageValue>> {
        todo!()
    }

    fn bytecode_by_hash(
        &self,
        code_hash: H256,
    ) -> std::result::Result<std::option::Option<reth_primitives::Bytecode>, reth_interfaces::Error>
    {
        todo!()
    }

    fn proof(
        &self,
        address: Address,
        keys: &[H256],
    ) -> RethResult<(Vec<Bytes>, H256, Vec<Vec<Bytes>>)> {
        todo!()
    }
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_call(
        &self,
        txs: Vec<EvmTransaction>,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/516
        let cfg_env = CfgEnv::default();
        let block_env = self.block_env.get(working_set).unwrap_or_default();

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        let chain_spec = Arc::new(ChainSpecBuilder::mainnet().berlin_activated().build());

        let db = SubState::new(State::new(InMemDb {}));

        let mut executor = Executor::new(chain_spec, db);

        let block: SealedBlockWithSenders = SealedBlockWithSenders {
            block: SealedBlock {
                header: SealedHeader {
                    header: todo!(),
                    hash: todo!(),
                },
                body: vec![],
                ommers: todo!(),
                withdrawals: todo!(),
            },
            senders: vec![],
        };
        let (block, senders) = block.into_components();
        let block = block.unseal();

        let post_state = executor.execute_and_verify_receipt(&block, U256::MAX, Some(senders))?;

        //todo!

        for tx in txs {
            self.transactions.set(&tx.hash, &tx, working_set);

            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
            let result =
                executor::execute_tx(evm_db, block_env.clone(), tx.clone(), cfg_env.clone())
                    .unwrap();

            let receipt = TransactionReceipt {
                transaction_hash: tx.hash,
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                transaction_index: 0,
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                block_hash: Default::default(),
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                block_number: Some(0),
                from: tx.sender,
                to: tx.to,
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                cumulative_gas_used: Default::default(),
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                gas_used: Default::default(),
                contract_address: contract_address(result).map(|addr| addr.into()),
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                status: Some(1),
                root: Default::default(),
                // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                transaction_type: Some(1),
                effective_gas_price: Default::default(),
            };

            self.receipts
                .set(&receipt.transaction_hash, &receipt, working_set);
        }
        Ok(CallResponse::default())
    }
}
