#[cfg(feature = "experimental")]
mod batch_builder;
#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_rpc, Ethereum};
#[cfg(feature = "experimental")]
pub use sov_evm::signer::DevSigner;

#[cfg(feature = "experimental")]
pub mod experimental {
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
    use reth_rpc::eth::error::EthApiError;
    use sov_evm::call::CallMessage;
    use sov_evm::evm::RlpEvmTransaction;
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::utils::to_jsonrpsee_error_object;
    use sov_modules_api::EncodeCall;
    use sov_rollup_interface::services::da::DaService;

    use super::batch_builder::EthBatchBuilder;
    use super::DevSigner;

    const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

    pub struct EthRpcConfig {
        pub min_blob_size: Option<usize>,
        pub sov_tx_signer_priv_key: DefaultPrivateKey,
        //TODO #839
        pub eth_signer: DevSigner,
    }

    pub fn get_ethereum_rpc<Da: DaService>(
        da_service: Da,
        eth_rpc_config: EthRpcConfig,
    ) -> RpcModule<Ethereum<Da>> {
        let mut rpc = RpcModule::new(Ethereum::new(
            Default::default(),
            da_service,
            Arc::new(Mutex::new(EthBatchBuilder::default())),
            eth_rpc_config,
        ));

        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct Ethereum<Da: DaService> {
        nonces: Mutex<HashMap<RethAddress, u64>>,
        da_service: Da,
        batch_builder: Arc<Mutex<EthBatchBuilder>>,
        eth_rpc_config: EthRpcConfig,
    }

    impl<Da: DaService> Ethereum<Da> {
        fn new(
            nonces: Mutex<HashMap<RethAddress, u64>>,
            da_service: Da,
            batch_builder: Arc<Mutex<EthBatchBuilder>>,
            eth_rpc_config: EthRpcConfig,
        ) -> Self {
            Self {
                nonces,
                da_service,
                batch_builder,
                eth_rpc_config,
            }
        }
    }

    impl<Da: DaService> Ethereum<Da> {
        fn make_raw_tx(
            &self,
            raw_tx: RlpEvmTransaction,
        ) -> Result<(H256, Vec<u8>), jsonrpsee::core::Error> {
            let signed_transaction: RethTransactionSignedNoHash =
                raw_tx.clone().try_into().map_err(EthApiError::from)?;

            let tx_hash = signed_transaction.hash();
            let sender = signed_transaction
                .recover_signer()
                .ok_or(EthApiError::InvalidTransactionSignature)?;

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

    fn register_rpc_methods<Da: DaService>(
        rpc: &mut RpcModule<Ethereum<Da>>,
    ) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method("eth_publishBatch", |params, ethereum| async move {
            let mut params_iter = params.sequence();

            let mut txs = Vec::default();
            while let Some(tx) = params_iter.optional_next::<Vec<u8>>()? {
                txs.push(tx)
            }

            let blob = ethereum
                .batch_builder
                .lock()
                .unwrap()
                .add_transactions_and_get_next_blob(Some(1), txs);

            if !blob.is_empty() {
                ethereum
                    .submit_batch(blob)
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;
            }
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

                let blob = ethereum
                    .batch_builder
                    .lock()
                    .unwrap()
                    .add_transactions_and_get_next_blob(
                        ethereum.eth_rpc_config.min_blob_size,
                        vec![raw_tx],
                    );

                if !blob.is_empty() {
                    ethereum
                        .submit_batch(blob)
                        .await
                        .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;
                }
                Ok::<_, ErrorObjectOwned>(tx_hash)
            },
        )?;

        rpc.register_async_method("eth_accounts", |_parameters, _ethereum| async move {
            #[allow(unreachable_code)]
            Ok::<_, ErrorObjectOwned>(todo!())
        })?;

        rpc.register_async_method("eth_estimateGas", |parameters, _ethereum| async move {
            let mut params = parameters.sequence();
            let _data: reth_rpc_types::CallRequest = params.next()?;
            let _block_number: Option<reth_primitives::BlockId> = params.optional_next()?;
            #[allow(unreachable_code)]
            Ok::<_, ErrorObjectOwned>(todo!())
        })?;

        Ok(())
    }
}
