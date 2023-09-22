#[cfg(feature = "experimental")]
mod batch_builder;
#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_rpc, Ethereum};
#[cfg(feature = "experimental")]
pub use sov_evm::signer::DevSigner;

#[cfg(feature = "experimental")]
pub mod experimental {
    use std::array::TryFromSliceError;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use borsh::ser::BorshSerialize;
    use demo_stf::app::DefaultPrivateKey;
    use demo_stf::runtime::{DefaultContext, Runtime};
    use ethers::types::{Bytes, H256};
    use jsonrpsee::types::ErrorObjectOwned;
    use jsonrpsee::RpcModule;
    use reth_primitives::{
        Address as RethAddress, TransactionSignedNoHash as RethTransactionSignedNoHash,
    };
    use reth_rpc_types::{TransactionRequest, TypedTransactionRequest};
    use sov_evm::call::CallMessage;
    use sov_evm::evm::RlpEvmTransaction;
    use sov_evm::Evm;
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::utils::to_jsonrpsee_error_object;
    use sov_modules_api::{EncodeCall, WorkingSet};
    use sov_rollup_interface::services::da::DaService;

    use super::batch_builder::EthBatchBuilder;
    #[cfg(feature = "local")]
    use super::DevSigner;

    const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

    pub struct EthRpcConfig {
        pub min_blob_size: Option<usize>,
        pub sov_tx_signer_priv_key: DefaultPrivateKey,
        #[cfg(feature = "local")]
        pub eth_signer: DevSigner,
    }

    pub fn get_ethereum_rpc<C: sov_modules_api::Context, Da: DaService>(
        da_service: Da,
        eth_rpc_config: EthRpcConfig,
        storage: C::Storage,
    ) -> RpcModule<Ethereum<C, Da>> {
        let mut rpc = RpcModule::new(Ethereum::new(
            Default::default(),
            da_service,
            Arc::new(Mutex::new(EthBatchBuilder::default())),
            eth_rpc_config,
            storage,
        ));

        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct Ethereum<C: sov_modules_api::Context, Da: DaService> {
        nonces: Mutex<HashMap<RethAddress, u64>>,
        da_service: Da,
        batch_builder: Arc<Mutex<EthBatchBuilder>>,
        eth_rpc_config: EthRpcConfig,
        storage: C::Storage,
    }

    impl<C: sov_modules_api::Context, Da: DaService> Ethereum<C, Da> {
        fn new(
            nonces: Mutex<HashMap<RethAddress, u64>>,
            da_service: Da,
            batch_builder: Arc<Mutex<EthBatchBuilder>>,
            eth_rpc_config: EthRpcConfig,
            storage: C::Storage,
        ) -> Self {
            Self {
                nonces,
                da_service,
                batch_builder,
                eth_rpc_config,
                storage,
            }
        }
    }

    impl<C: sov_modules_api::Context, Da: DaService> Ethereum<C, Da> {
        fn make_raw_tx(
            &self,
            raw_tx: RlpEvmTransaction,
        ) -> Result<(H256, Vec<u8>), jsonrpsee::core::Error> {
            let signed_transaction: RethTransactionSignedNoHash = raw_tx.clone().try_into()?;

            let tx_hash = signed_transaction.hash();
            let sender = signed_transaction.recover_signer().ok_or(
                sov_evm::evm::primitive_types::RawEvmTxConversionError::FailedToRecoverSigner,
            )?;

            let mut nonces = self.nonces.lock().unwrap();
            let nonce = *nonces.entry(sender).and_modify(|n| *n += 1).or_insert(0);

            let tx = CallMessage { tx: raw_tx };
            let message = <Runtime<DefaultContext, Da::Spec> as EncodeCall<
                sov_evm::Evm<DefaultContext>,
            >>::encode_call(tx);

            let tx = Transaction::<DefaultContext>::new_signed_tx(
                &self.eth_rpc_config.sov_tx_signer_priv_key,
                message,
                nonce,
            );
            Ok((H256::from(tx_hash), tx.try_to_vec()?))
        }

        async fn build_and_submit_batch(
            &self,
            txs: Vec<Vec<u8>>,
            min_blob_size: Option<usize>,
        ) -> Result<(), jsonrpsee::core::Error> {
            let min_blob_size = min_blob_size.or(self.eth_rpc_config.min_blob_size);

            let batch = self
                .batch_builder
                .lock()
                .unwrap()
                .add_transactions_and_get_next_blob(min_blob_size, txs);

            if !batch.is_empty() {
                self.submit_batch(batch)
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;
            }
            Ok(())
        }

        async fn submit_batch(&self, raw_txs: Vec<Vec<u8>>) -> Result<(), jsonrpsee::core::Error> {
            let blob = raw_txs
                .try_to_vec()
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            self.da_service
                .send_transaction(&blob)
                .await
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            Ok(())
        }
    }

    fn register_rpc_methods<C: sov_modules_api::Context, Da: DaService>(
        rpc: &mut RpcModule<Ethereum<C, Da>>,
    ) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method("eth_publishBatch", |params, ethereum| async move {
            let mut params_iter = params.sequence();

            let mut txs = Vec::default();
            while let Some(tx) = params_iter.optional_next::<Vec<u8>>()? {
                txs.push(tx)
            }

            ethereum
                .build_and_submit_batch(txs, Some(1))
                .await
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            Ok::<String, ErrorObjectOwned>("Submitted transaction".to_string())
        })?;

        rpc.register_async_method(
            "eth_sendRawTransaction",
            |parameters, ethereum| async move {
                let data: Bytes = parameters.one().unwrap();

                let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };

                let (tx_hash, raw_tx) = ethereum
                    .make_raw_tx(raw_evm_tx)
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                ethereum
                    .build_and_submit_batch(vec![raw_tx], None)
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                Ok::<_, ErrorObjectOwned>(tx_hash)
            },
        )?;

        #[cfg(feature = "local")]
        rpc.register_async_method("eth_accounts", |_parameters, ethereum| async move {
            Ok::<_, ErrorObjectOwned>(ethereum.eth_rpc_config.eth_signer.signers())
        })?;

        #[cfg(feature = "local")]
        rpc.register_async_method("eth_sendTransaction", |parameters, ethereum| async move {
            let mut transaction_request: TransactionRequest = parameters.one().unwrap();

            let evm = Evm::<C>::default();

            // get from, return error if none
            let from = transaction_request
                .from
                .ok_or(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR))?;

            // return error if not in signers
            if !ethereum.eth_rpc_config.eth_signer.signers().contains(&from) {
                return Err(to_jsonrpsee_error_object(
                    "From address not in signers",
                    ETH_RPC_ERROR,
                ));
            }

            let raw_evm_tx = {
                let mut working_set = WorkingSet::<C>::new(ethereum.storage.clone());
                if transaction_request.nonce.is_none() {
                    let nonce = evm
                        .get_transaction_count(from, None, &mut working_set)
                        .unwrap_or_default();

                    transaction_request.nonce = Some(nonce);
                }

                let chain_id = evm
                    .chain_id(&mut working_set)
                    .expect("Failed to get chain id")
                    .map(|id| id.as_u64())
                    .unwrap_or(1);

                // TODO: implement gas logic after gas estimation (#906) is implemented
                // https://github.com/Sovereign-Labs/sovereign-sdk/issues/906
                let transaction_request = match transaction_request.into_typed_request() {
                    Some(TypedTransactionRequest::Legacy(mut m)) => {
                        m.chain_id = Some(chain_id);

                        TypedTransactionRequest::Legacy(m)
                    }
                    Some(TypedTransactionRequest::EIP2930(mut m)) => {
                        m.chain_id = chain_id;

                        TypedTransactionRequest::EIP2930(m)
                    }
                    Some(TypedTransactionRequest::EIP1559(mut m)) => {
                        m.chain_id = chain_id;

                        TypedTransactionRequest::EIP1559(m)
                    }
                    None => {
                        return Err(to_jsonrpsee_error_object(
                            "Conflicting fee fields",
                            ETH_RPC_ERROR,
                        ));
                    }
                };

                let transaction = into_transaction(transaction_request).map_err(|_| {
                    to_jsonrpsee_error_object("Invalid types in transaction request", ETH_RPC_ERROR)
                })?;

                let signed_tx = ethereum
                    .eth_rpc_config
                    .eth_signer
                    .sign_transaction(transaction, from)
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                RlpEvmTransaction {
                    rlp: signed_tx.envelope_encoded().to_vec(),
                }
            };
            let (tx_hash, raw_tx) = ethereum
                .make_raw_tx(raw_evm_tx)
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            ethereum
                .build_and_submit_batch(vec![raw_tx], None)
                .await
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            Ok::<_, ErrorObjectOwned>(tx_hash)
        })?;

        Ok(())
    }

    // Temporary solution until https://github.com/paradigmxyz/reth/issues/4704 is resolved
    // The problem is having wrong length nonce/gas_limt/value fields in the transaction request
    fn into_transaction(
        request: TypedTransactionRequest,
    ) -> Result<reth_primitives::Transaction, TryFromSliceError> {
        Ok(match request {
            TypedTransactionRequest::Legacy(tx) => {
                reth_primitives::Transaction::Legacy(reth_primitives::TxLegacy {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce.as_u64(),
                    gas_price: u128::from_be_bytes(tx.gas_price.to_be_bytes()),
                    gas_limit: convert_u256_to_u64(tx.gas_limit)?,
                    to: tx.kind.into(),
                    value: convert_u256_to_u128(tx.value)?,
                    input: tx.input,
                })
            }
            TypedTransactionRequest::EIP2930(tx) => {
                reth_primitives::Transaction::Eip2930(reth_primitives::TxEip2930 {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce.as_u64(),
                    gas_price: u128::from_be_bytes(tx.gas_price.to_be_bytes()),
                    gas_limit: convert_u256_to_u64(tx.gas_limit)?,
                    to: tx.kind.into(),
                    value: convert_u256_to_u128(tx.value)?,
                    input: tx.input,
                    access_list: tx.access_list,
                })
            }
            TypedTransactionRequest::EIP1559(tx) => {
                reth_primitives::Transaction::Eip1559(reth_primitives::TxEip1559 {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce.as_u64(),
                    max_fee_per_gas: u128::from_be_bytes(tx.max_fee_per_gas.to_be_bytes()),
                    gas_limit: convert_u256_to_u64(tx.gas_limit)?,
                    to: tx.kind.into(),
                    value: convert_u256_to_u128(tx.value)?,
                    input: tx.input,
                    access_list: tx.access_list,
                    max_priority_fee_per_gas: u128::from_be_bytes(
                        tx.max_priority_fee_per_gas.to_be_bytes(),
                    ),
                })
            }
        })
    }

    fn convert_u256_to_u64(u256: reth_primitives::U256) -> Result<u64, TryFromSliceError> {
        let bytes: [u8; 32] = u256.to_be_bytes();
        let bytes: [u8; 8] = bytes[24..].try_into()?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn convert_u256_to_u128(u256: reth_primitives::U256) -> Result<u128, TryFromSliceError> {
        let bytes: [u8; 32] = u256.to_be_bytes();
        let bytes: [u8; 16] = bytes[16..].try_into()?;
        Ok(u128::from_be_bytes(bytes))
    }
}
