#[cfg(feature = "experimental")]
mod batch_builder;
pub use batch_builder::EthBatchBuilder;
#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_rpc, Ethereum};

#[cfg(feature = "experimental")]
pub mod experimental {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use borsh::ser::BorshSerialize;
    use demo_stf::app::DefaultPrivateKey;
    use demo_stf::runtime::{DefaultContext, Runtime};
    use ethers::types::{Bytes, H256};
    use jsonrpsee::types::ErrorObjectOwned;
    use jsonrpsee::RpcModule;
    use reth_primitives::TransactionSignedNoHash as RethTransactionSignedNoHash;
    use reth_rpc::eth::error::EthApiError;
    use sov_evm::call::CallMessage;
    use sov_evm::evm::{EthAddress, RawEvmTransaction};
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::utils::to_jsonrpsee_error_object;
    use sov_modules_api::EncodeCall;
    use sov_rollup_interface::services::batch_builder::{self, BatchBuilder};
    use sov_rollup_interface::services::da::DaService;

    const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

    pub fn get_ethereum_rpc<B: BatchBuilder + Send + Sync + 'static, DA: DaService>(
        da_service: DA,
        tx_signer_prov_key: DefaultPrivateKey,
        batch_builder: B,
    ) -> RpcModule<Ethereum<B, DA>> {
        let mut rpc = RpcModule::new(Ethereum {
            nonces: Default::default(),
            tx_signer_prov_key,
            da_service,
            batch_builder,
        });
        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct Ethereum<B: BatchBuilder, DA: DaService> {
        nonces: Mutex<HashMap<EthAddress, u64>>,
        tx_signer_prov_key: DefaultPrivateKey,
        da_service: DA,
        batch_builder: B,
    }

    impl<B: BatchBuilder, DA: DaService> Ethereum<B, DA> {
        fn make_raw_tx(
            &self,
            raw_tx: RawEvmTransaction,
        ) -> Result<(H256, Vec<u8>), jsonrpsee::core::Error> {
            let signed_transaction: RethTransactionSignedNoHash =
                raw_tx.clone().try_into().map_err(EthApiError::from)?;

            let tx_hash = signed_transaction.hash();
            let sender = signed_transaction
                .recover_signer()
                .ok_or(EthApiError::InvalidTransactionSignature)?;

            let mut nonces = self.nonces.lock().unwrap();
            let nonce = *nonces
                .entry(sender.into())
                .and_modify(|n| *n += 1)
                .or_insert(0);

            let tx = CallMessage { tx: raw_tx };
            let message =
                <Runtime<DefaultContext> as EncodeCall<sov_evm::Evm<DefaultContext>>>::encode_call(
                    tx,
                );

            let tx = Transaction::<DefaultContext>::new_signed_tx(
                &self.tx_signer_prov_key,
                message,
                nonce,
            );
            Ok((H256::from(tx_hash), tx.try_to_vec()?))
        }
    }

    fn register_rpc_methods<B: BatchBuilder + Send + Sync + 'static, DA: DaService>(
        rpc: &mut RpcModule<Ethereum<B, DA>>,
    ) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method(
            "eth_sendRawTransaction",
            |parameters, ethereum| async move {
                let data: Bytes = parameters.one().unwrap();

                let raw_evm_tx = RawEvmTransaction { rlp: data.to_vec() };
                let (tx_hash, raw_tx) = ethereum
                    .make_raw_tx(raw_evm_tx)
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                let blob = vec![raw_tx]
                    .try_to_vec()
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                ethereum
                    .da_service
                    .send_transaction(&blob)
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                Ok::<_, ErrorObjectOwned>(tx_hash)
            },
        )?;

        Ok(())
    }
}
