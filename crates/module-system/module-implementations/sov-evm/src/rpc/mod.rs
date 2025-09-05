use alloy_consensus::{Transaction as TransactionTrait, TxReceipt};
use alloy_primitives::{Address, U64};
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, Block, BlockOverrides, BlockTransactions, FeeHistory, Log,
    ReceiptEnvelope, ReceiptWithBloom, Transaction, TransactionReceipt, TransactionRequest,
};
use error::ensure_success;
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use reth_primitives::{Recovered, TransactionSigned};
use reth_rpc_eth_types::{EthApiError, RpcInvalidTransactionError};
use revm::context::result::{EVMError, InvalidHeader};
use revm::context::{BlockEnv, CfgEnv};
use revm::Database;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, Spec, StateAccessor};
use tracing::debug;

use crate::db::EvmDb;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, SealedBlock, TransactionSignedAndRecovered};
use crate::executor::get_cfg_env;
use crate::helpers::{
    from_primitive_with_hash, from_recovered_with_block_context, prepare_call_env,
};
use crate::primitive_types::MaybeSealedBlock;
use crate::Evm;

pub(crate) mod error;

#[rpc_gen(client, server)]
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Handler for `net_version`
    #[rpc_method(name = "net_version")]
    pub fn net_version(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        debug!("EVM module JSON-RPC request to `net_version`");

        // Network ID is the same as chain ID for most networks
        let chain_id = self.cfg_infallible(state).chain_spec.chain_id;
        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id = self.cfg_infallible(state).chain_spec.chain_id;
        debug!(
            chain_id = chain_id,
            "EVM module JSON-RPC request to `eth_chainId`"
        );
        Ok(Some(U64::from(chain_id)))
    }

    /// Handler for `eth_getBlockByHash`
    #[rpc_method(name = "eth_getBlockByHash")]
    pub fn get_block_by_hash(
        &self,
        block_hash: B256,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Block>> {
        debug!(
            ?block_hash,
            "EVM module JSON-RPC request to `eth_getBlockByHash`"
        );

        let block_number_hex = self
            .block_hashes
            .get(&block_hash, state)
            .unwrap_infallible()
            .map(|number| hex::encode(number.to_be_bytes()));

        match block_number_hex {
            Some(block_number_hex) => {
                self.get_block_by_number(Some(block_number_hex), details, state)
            }
            None => Ok(None),
        }
    }

    /// Handler for: `eth_getBlockByNumber`
    #[rpc_method(name = "eth_getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        block_number: Option<String>,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Block>> {
        debug!(
            block_number,
            "EVM module JSON-RPC request to `eth_getBlockByNumber`"
        );

        let maybe_block = || -> Option<Block> {
            let block = self.get_sealed_block_by_number(block_number, state)?;
            let header = from_primitive_with_hash(block.header.clone());

            let transactions = if Some(true) == details {
                BlockTransactions::Full(
                    block
                        .transactions
                        .clone()
                        .map(|index| {
                            let tx = self.transactions.get(&index, state).unwrap_infallible()?;
                            Some(from_recovered_with_block_context(
                                tx.into(),
                                block.header.seal(),
                                block.header.number,
                                block.header.base_fee_per_gas,
                                U256::from(index - block.transactions.start),
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?,
                )
            } else {
                BlockTransactions::Hashes(
                    block
                        .transactions
                        .clone()
                        .map(|index| {
                            let tx = self.transactions.get(&index, state).unwrap_infallible()?;
                            Some(*tx.signed_transaction.hash())
                        })
                        .collect::<Option<Vec<_>>>()?,
                )
            };

            Some(Block {
                header,
                transactions,
                ..Default::default()
            })
        };

        Ok(maybe_block())
    }

    /// Handler for: `eth_getBalance`
    #[rpc_method(name = "eth_getBalance")]
    pub fn get_balance(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let balance = self
            .get_db(state)
            .basic(address)
            .map_err(EthApiError::from)?
            .map(|account| account.balance)
            .unwrap_or_default();

        debug!(
            %address,
            %balance,
            "EVM module JSON-RPC request to `eth_getBalance`"
        );

        Ok(balance)
    }

    /// Handler for: `eth_getStorageAt`
    #[rpc_method(name = "eth_getStorageAt")]
    pub fn get_storage_at(
        &self,
        address: Address,
        index: U256,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        debug!("EVM module JSON-RPC request to `eth_getStorageAt`");

        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let storage_slot = self
            .account_storage
            .get(&(&address, &index), state)
            .unwrap_infallible()
            .unwrap_or_default();

        Ok(storage_slot)
    }

    /// Handler for: `eth_getTransactionCount`
    #[rpc_method(name = "eth_getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        // TODO: Implement block_number once we have archival state #882
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/882

        let ethereum_address: EthereumAddress = address.into();
        let credential_id = ethereum_address.as_credential_id();

        let nonce = self
            .uniqueness_module
            .next_nonce(&credential_id, state)
            .unwrap_or_default();

        debug!(%address, nonce, "EVM module JSON-RPC request to `eth_getTransactionCount`");
        Ok(U64::from(nonce))
    }

    /// Handler for: `eth_getCode`
    #[rpc_method(name = "eth_getCode")]
    pub fn get_code(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_getCode`");

        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let code = self
            .accounts
            .get(&address, state)
            .unwrap_infallible()
            .and_then(|account| self.code.get(&account.code_hash, state).unwrap_infallible())
            .unwrap_or_default();

        Ok(code)
    }

    /// Handler for: `eth_feeHistory`
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "eth_feeHistory")]
    pub fn fee_history(&self) -> RpcResult<FeeHistory> {
        debug!("EVM module JSON-RPC request to `eth_feeHistory`");

        Ok(FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
            blob_gas_used_ratio: Default::default(),
            // EIP-4844 related
            base_fee_per_blob_gas: Default::default(),
        })
    }

    /// Handler for: `eth_getTransactionByHash`
    #[rpc_method(name = "eth_getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<Transaction>> {
        let mut maybe_tx = || -> Option<Transaction> {
            let tx_number = self.get_tx_index_by_hash(&hash, state)?;
            let tx = self.transaction(tx_number, state)?;
            let block = self.block(tx.block_number, state)?;

            Some(from_recovered_with_block_context(
                tx.into(),
                block.header.seal(),
                block.header.number,
                block.header.base_fee_per_gas,
                U256::from(tx_number - block.transactions.start),
            ))
        };

        let transaction = maybe_tx();
        debug!(
            %hash,
            ?transaction,
            "EVM module JSON-RPC request to `eth_getTransactionByHash`"
        );

        Ok(transaction)
    }

    /// Handler for: `eth_getTransactionReceipt`
    #[rpc_method(name = "eth_getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<TransactionReceipt>> {
        debug!(
            %hash,
            "EVM module JSON-RPC request to `eth_getTransactionReceipt`"
        );

        let mut maybe_receipt = || -> Option<TransactionReceipt> {
            let number = self.get_tx_index_by_hash(&hash, state)?;
            let tx = self.transaction(number, state)?;
            let block = self.get_maybe_sealed_block(&tx, state);
            let receipt = self.receipt(number, state)?;
            Some(build_rpc_receipt(block, tx, number, receipt))
        };

        Ok(maybe_receipt())
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    #[rpc_method(name = "eth_call")]
    pub fn eth_call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_call`");

        let Some(block_env) = self.resolve_block_env(block_number, state) else {
            return Err(eth_api_into_rpc_error(EthApiError::UnknownBlockOrTxIndex));
        };

        let tx_env =
            prepare_call_env(&block_env, request.clone()).map_err(eth_api_into_rpc_error)?;

        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, cfg, Some(get_cfg_env_template()));

        let evm_db: EvmDb<_, S> = self.get_db(state);

        let result = match executor::call(evm_db, &block_env, tx_env, cfg_env) {
            Ok(result) => result,
            Err(err) => return Err(eth_api_into_rpc_error(eth_from_evm_error(err))),
        };

        ensure_success(result).map_err(eth_api_into_rpc_error)
    }

    /// Handler for: `eth_blockNumber`
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        let block_number_range = self
            .block_numbers
            .get(state)
            .unwrap_infallible()
            // Justified, we set it at genesis and later only override it.
            .expect("The impossible happened: block_numbers was not set.");

        Ok(U256::from(*block_number_range.end()))
    }

    /// Handler for: `eth_estimateGas`
    // https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/api/call.rs#L172
    #[rpc_method(name = "eth_estimateGas")]
    pub fn eth_estimate_gas(
        &self,
        _request: TransactionRequest,
        _block_number: Option<String>,
        _state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        // TODO EVM: #1510
        Ok(U64::from(100_000))
    }
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn get_maybe_sealed_block(
        &self,
        tx: &TransactionSignedAndRecovered,
        state: &mut ApiStateAccessor<S>,
    ) -> MaybeSealedBlock {
        let block = self.blocks.get(&tx.block_number, state).unwrap_infallible();
        if let Some(block) = block {
            return MaybeSealedBlock::Sealed(block.into());
        }
        let current_block_env = self
            .block_env
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default();
        let block_num: u64 = current_block_env.number.try_into().expect(
            "The impossible happened: block number is too large to fit in a u64. It's over!",
        );
        assert_eq!(block_num, tx.block_number, "Transaction is in a block that is not yet sealed, but that block is not yet pending! This is impossible!");

        let head = self
            .head
            .get(state)
            .unwrap_infallible()
            // Justified, the head is initialized at genesis and modified only later through overrides.
            .expect("The impossible happened: head was not set.");
        let first_tx_index = head.transactions.end;

        MaybeSealedBlock::Pending {
            block_number: tx.block_number,
            first_tx_number: first_tx_index,
            base_fee_per_gas: current_block_env.basefee,
        }
    }

    fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<SealedBlock> {
        // safe, finalized, and pending are not supported
        match block_number {
            Some(ref block_number) if block_number == "earliest" => {
                let block_numbers = self
                    .block_numbers
                    .get(state)
                    .unwrap_infallible()
                    // This is justified, as block numbers are set at genesis and only overridden later.
                    .expect("The impossible happened: block_numbers was not set.");
                let first_block_number = block_numbers.start();

                self.blocks
                    .get(first_block_number, state)
                    .unwrap_infallible()
            }
            Some(ref block_number) if block_number == "latest" => {
                let block_numbers = self
                    .block_numbers
                    .get(state)
                    .unwrap_infallible()
                    // This is justified, as block numbers are set at genesis and only overridden later.
                    .expect("The impossible happened: block_numbers was not set.");

                let last_block_number = block_numbers.end();

                self.blocks
                    .get(last_block_number, state)
                    .unwrap_infallible()
            }
            Some(ref block_number) => {
                // hex representation may have 0x prefix
                let Ok(block_number) =
                    u64::from_str_radix(block_number.trim_start_matches("0x"), 16)
                else {
                    tracing::error!(
                        block_number,
                        "get_sealed_block_by_number: Block number must be a valid hex number, with or without 0x prefix"
                    );

                    return None;
                };

                self.blocks.get(&block_number, state).unwrap_infallible()
            }
            None => self.get_sealed_block_by_number(Some("latest".into()), state),
        }
    }

    fn resolve_block_env(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<BlockEnv> {
        match block_number {
            Some(ref block_number) if block_number == "pending" => {
                self.block_env.get(state).unwrap_infallible()
            }
            _ => {
                let block = self.get_sealed_block_by_number(block_number, state)?;
                Some(BlockEnv::from(block))
            }
        }
    }
}

fn get_cfg_env_template() -> CfgEnv {
    let mut cfg_env = CfgEnv::default();
    // Reth sets this to true and uses only timeout, but other clients use this as a part of DOS attacks protection, with 100mln gas limit
    // https://github.com/paradigmxyz/reth/blob/62f39a5a151c5f4ddc9bf0851725923989df0412/crates/rpc/rpc/src/eth/revm_utils.rs#L215
    cfg_env.disable_block_gas_limit = false;
    cfg_env.disable_eip3607 = true;
    cfg_env.disable_base_fee = true;
    cfg_env.chain_id = config_value!("CHAIN_ID");
    cfg_env.limit_contract_code_size = None;
    cfg_env
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: MaybeSealedBlock,
    tx: TransactionSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
) -> TransactionReceipt {
    let transaction: Recovered<TransactionSigned> = tx.into();
    let from = transaction.signer();

    let block_hash = block.hash();
    let block_number = Some(block.number());
    let transaction_hash = Some(*transaction.hash());
    // Safety: The transaction cannot have a lower number than the block start
    let transaction_index = tx_number
        .checked_sub(block.transactions_start())
        .expect("The impossible happened: overflow while subtracting block start from tx number.");

    let logs: Vec<Log> = receipt
        .receipt
        .logs
        .iter()
        .enumerate()
        .map(|(tx_log_idx, log)| Log {
            inner: log.clone(),
            block_hash,
            block_number,
            block_timestamp: block.timestamp(),
            transaction_hash,
            transaction_index: Some(transaction_index),
            log_index: Some(receipt.log_index_start + tx_log_idx as u64),
            removed: false,
        })
        .collect();

    let logs_bloom = receipt.receipt.bloom();

    let rpc_receipt = alloy_rpc_types::Receipt {
        status: receipt.receipt.success.into(),
        cumulative_gas_used: receipt.receipt.cumulative_gas_used,
        logs,
    };

    let (contract_address, to) = match transaction.kind() {
        TxKind::Create => (Some(from.create(transaction.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(ReceiptWithBloom::new(rpc_receipt, logs_bloom)),
        transaction_hash: *transaction.hash(),
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used,
        effective_gas_price: transaction.effective_gas_price(Some(block.base_fee_per_gas())),
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
    }
}

fn eth_from_evm_error<Ws: StateAccessor>(err: EVMError<crate::db::Error<Ws>>) -> EthApiError {
    match err {
        EVMError::Transaction(err) => RpcInvalidTransactionError::from(err).into(),
        EVMError::Header(InvalidHeader::PrevrandaoNotSet) => EthApiError::PrevrandaoNotSet,
        EVMError::Header(InvalidHeader::ExcessBlobGasNotSet) => EthApiError::ExcessBlobGasNotSet,
        EVMError::Database(db_err) => db_err.into(),
        EVMError::Custom(data) => EthApiError::EvmCustom(data),
    }
}

impl<Ws: StateAccessor> From<crate::db::Error<Ws>> for EthApiError {
    fn from(err: crate::db::Error<Ws>) -> Self {
        RpcInvalidTransactionError::other(ErrorObject::owned(
            -32603,
            format!("Database error: {err}"),
            None::<()>,
        ))
        .into()
    }
}

/// Hack while reth is not upgraded for `jsonrpsee` 0.25
pub fn eth_api_into_rpc_error(eth_error: EthApiError) -> ErrorObjectOwned {
    ErrorObject::owned(500, format!("Eth Error: {eth_error:?}"), None::<()>)
}

/// Hack while reth is not upgraded for `jsonrpsee` 0.25
pub fn invalid_tx_into_rpc_error(rpc: RpcInvalidTransactionError) -> ErrorObjectOwned {
    eth_api_into_rpc_error(rpc.into())
}
