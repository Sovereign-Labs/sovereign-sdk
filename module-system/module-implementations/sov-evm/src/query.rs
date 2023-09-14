use ethereum_types::U64;
use jsonrpsee::core::RpcResult;
use reth_primitives::contract::create_address;
use reth_primitives::TransactionKind::{Call, Create};
use reth_primitives::{TransactionSignedEcRecovered, U128, U256};
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;
use tracing::info;

use crate::call::get_cfg_env;
use crate::evm::db::EvmDb;
use crate::evm::primitive_types::{Receipt, SealedBlock, TransactionSignedAndRecovered};
use crate::evm::{executor, prepare_call_env};
use crate::Evm;

#[rpc_gen(client, server, namespace = "eth")]
impl<C: sov_modules_api::Context> Evm<C> {
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "chainId")]
    pub fn chain_id(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_primitives::U64>> {
        info!("evm module: eth_chainId");
        Ok(Some(reth_primitives::U64::from(1u64)))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        _block_number: Option<String>,
        _details: Option<bool>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::RichBlock>> {
        info!("evm module: eth_getBlockByNumber");

        let header = reth_rpc_types::Header {
            hash: Default::default(),
            parent_hash: Default::default(),
            uncles_hash: Default::default(),
            miner: Default::default(),
            state_root: Default::default(),
            transactions_root: Default::default(),
            receipts_root: Default::default(),
            logs_bloom: Default::default(),
            difficulty: Default::default(),
            number: Default::default(),
            gas_limit: Default::default(),
            gas_used: Default::default(),
            timestamp: Default::default(),
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee_per_gas: Some(reth_primitives::U256::from(100)),
            withdrawals_root: Default::default(),
        };

        let block = reth_rpc_types::Block {
            header,
            total_difficulty: Default::default(),
            uncles: Default::default(),
            transactions: reth_rpc_types::BlockTransactions::Hashes(Default::default()),
            size: Default::default(),
            withdrawals: Default::default(),
        };

        Ok(Some(block.into()))
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "feeHistory")]
    pub fn fee_history(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_rpc_types::FeeHistory> {
        info!("evm module: eth_feeHistory");
        Ok(reth_rpc_types::FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
        })
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: reth_primitives::H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::Transaction>> {
        info!("evm module: eth_getTransactionByHash");
        let mut accessory_state = working_set.accessory_state();

        let tx_number = self.transaction_hashes.get(&hash, &mut accessory_state);

        let transaction = tx_number.map(|number| {
            let tx = self
                .transactions
                .get(number as usize, &mut accessory_state)
                .unwrap_or_else(|| panic!("Transaction with known hash {} and number {} must be set in all {} transaction",                
                hash,
                number,
                self.transactions.len(&mut accessory_state)));

            let block = self
                .blocks
                .get(tx.block_number as usize, &mut accessory_state)
                .unwrap_or_else(|| panic!("Block with number {} for known transaction {} must be set",
                    tx.block_number,
                    tx.signed_transaction.hash));

            reth_rpc_types::Transaction::from_recovered_with_block_context(
                tx.into(),
                block.header.hash,
                block.header.number,
                block.header.base_fee_per_gas,
                U256::from(tx_number.unwrap() - block.transactions.start),
            )
        });

        Ok(transaction)
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: reth_primitives::H256,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<Option<reth_rpc_types::TransactionReceipt>> {
        info!("evm module: eth_getTransactionReceipt");

        let mut accessory_state = working_set.accessory_state();

        let tx_number = self.transaction_hashes.get(&hash, &mut accessory_state);

        let receipt = tx_number.map(|number| {
            let tx = self
                .transactions
                .get(number as usize, &mut accessory_state)
                .expect("Transaction with known hash must be set");
            let block = self
                .blocks
                .get(tx.block_number as usize, &mut accessory_state)
                .expect("Block number for known transaction must be set");

            let receipt = self
                .receipts
                .get(tx_number.unwrap() as usize, &mut accessory_state)
                .expect("Receipt for known transaction must be set");

            build_rpc_receipt(block, tx, tx_number.unwrap(), receipt)
        });

        Ok(receipt)
    }

    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    //https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-types/src/eth/call.rs#L7

    /// Template env for eth_call
    const CALL_CFG_ENV_TEMPLATE: revm::primitives::CfgEnv = revm::primitives::CfgEnv {
        // Reth sets this to true and uses only timeout, but other clients use this as a part of DOS attacks protection, with 100mln gas limit
        // https://github.com/paradigmxyz/reth/blob/62f39a5a151c5f4ddc9bf0851725923989df0412/crates/rpc/rpc/src/eth/revm_utils.rs#L215
        disable_block_gas_limit: false,
        disable_eip3607: true,
        disable_base_fee: true,
        chain_id: revm::primitives::U256::ZERO,
        spec_id: revm::primitives::SpecId::LATEST,
        perf_all_precompiles_have_balance: false,
        perf_analyse_created_bytecodes: revm::primitives::AnalysisKind::Analyse,
        limit_contract_code_size: None,
    };

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "call")]
    pub fn get_call(
        &self,
        request: reth_rpc_types::CallRequest,
        _block_number: Option<reth_primitives::BlockId>,
        _state_overrides: Option<reth_rpc_types::state::StateOverride>,
        _block_overrides: Option<Box<reth_rpc_types::BlockOverrides>>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::Bytes> {
        info!("evm module: eth_call");
        let tx_env = prepare_call_env(request);

        let block_env = self.pending_block.get(working_set).unwrap_or_default();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let cfg_env = get_cfg_env(&block_env, cfg, Some(Self::CALL_CFG_ENV_TEMPLATE));

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::inspect(evm_db, &block_env, tx_env, cfg_env).unwrap();
        let output = match result.result {
            revm::primitives::ExecutionResult::Success { output, .. } => output,
            _ => todo!(),
        };
        Ok(output.into_data().into())
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "sendTransaction")]
    pub fn send_transaction(
        &self,
        _request: reth_rpc_types::TransactionRequest,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::U256> {
        unimplemented!("eth_sendTransaction not implemented")
    }

    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "blockNumber")]
    pub fn block_number(
        &self,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::U256> {
        unimplemented!("eth_blockNumber not implemented")
    }

    #[rpc_method(name = "estimateGas")]
    pub fn eth_estimate_gas(
        &self,
        _data: reth_rpc_types::CallRequest,
        _block_number: Option<reth_primitives::BlockId>,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<reth_primitives::U256> {
        unimplemented!("eth_sendTransaction not implemented")
    }
}

// modified from: https://github.com/paradigmxyz/reth/blob/cc576bc8690a3e16e6e5bf1cbbbfdd029e85e3d4/crates/rpc/rpc/src/eth/api/transactions.rs#L849
pub(crate) fn build_rpc_receipt(
    block: SealedBlock,
    tx: TransactionSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
) -> reth_rpc_types::TransactionReceipt {
    let transaction: TransactionSignedEcRecovered = tx.into();
    let transaction_kind = transaction.kind();

    let transaction_hash = Some(transaction.hash);
    let transaction_index = Some(U256::from(tx_number - block.transactions.start));
    let block_hash = Some(block.header.hash);
    let block_number = Some(U256::from(block.header.number));

    reth_rpc_types::TransactionReceipt {
        transaction_hash,
        transaction_index,
        block_hash,
        block_number,
        from: transaction.signer(),
        to: match transaction_kind {
            Create => None,
            Call(addr) => Some(*addr),
        },
        cumulative_gas_used: U256::from(receipt.receipt.cumulative_gas_used),
        gas_used: Some(U256::from(receipt.gas_used)),
        contract_address: match transaction_kind {
            Create => Some(create_address(transaction.signer(), transaction.nonce())),
            Call(_) => None,
        },
        effective_gas_price: U128::from(
            transaction.effective_gas_price(block.header.base_fee_per_gas),
        ),
        transaction_type: transaction.tx_type().into(),
        logs_bloom: receipt.receipt.bloom_slow(),
        status_code: if receipt.receipt.success {
            Some(U64::from(1))
        } else {
            Some(U64::from(0))
        },
        state_root: None, // Pre https://eips.ethereum.org/EIPS/eip-658 (pre-byzantium) and won't be used
        logs: receipt
            .receipt
            .logs
            .into_iter()
            .enumerate()
            .map(|(idx, log)| reth_rpc_types::Log {
                address: log.address,
                topics: log.topics,
                data: log.data,
                block_hash,
                block_number,
                transaction_hash,
                transaction_index,
                log_index: Some(U256::from(receipt.log_index_start + idx as u64)),
                removed: false,
            })
            .collect(),
    }
}
