#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_rpc, Ethereum};

#[cfg(feature = "experimental")]
pub mod experimental {

    use std::collections::HashMap;
    use std::sync::Mutex;

    use borsh::ser::BorshSerialize;
    use const_rollup_config::ROLLUP_NAMESPACE_RAW;
    use demo_stf::app::DefaultPrivateKey;
    use demo_stf::runtime::{DefaultContext, Runtime};
    use ethers::types::{Bytes, H256};
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::core::params::ArrayParams;
    use jsonrpsee::http_client::{HeaderMap, HttpClient};
    use jsonrpsee::types::ErrorObjectOwned;
    use jsonrpsee::RpcModule;
    use jupiter::da_service::DaServiceConfig;
    use reth_primitives::TransactionSignedNoHash as RethTransactionSignedNoHash;
    use reth_rpc::eth::error::EthApiError;
    use sov_evm::call::CallMessage;
    use sov_evm::evm::{EthAddress, RawEvmTransaction};
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::utils::to_jsonrpsee_error_object;

    const GAS_PER_BYTE: usize = 120;
    const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

    pub fn get_ethereum_rpc(
        config: DaServiceConfig,
        tx_signer_prov_key: DefaultPrivateKey,
    ) -> RpcModule<Ethereum> {
        let mut rpc = RpcModule::new(Ethereum {
            config,
            nonces: Default::default(),
            tx_signer_prov_key,
        });
        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct Ethereum {
        config: DaServiceConfig,
        nonces: Mutex<HashMap<EthAddress, u64>>,
        tx_signer_prov_key: DefaultPrivateKey,
    }

    impl Ethereum {
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
            let message = Runtime::<DefaultContext>::encode_evm_call(tx);
            let tx = Transaction::<DefaultContext>::new_signed_tx(
                &self.tx_signer_prov_key,
                message,
                nonce,
            );
            Ok((H256::from(tx_hash), tx.try_to_vec()?))
        }
    }

    impl Ethereum {
        fn make_client(&self) -> HttpClient {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Authorization",
                format!("Bearer {}", self.config.celestia_rpc_auth_token.clone())
                    .parse()
                    .unwrap(),
            );

            jsonrpsee::http_client::HttpClientBuilder::default()
                .set_headers(headers)
                .max_request_size(default_max_response_size())
                .build(self.config.celestia_rpc_address.clone())
                .expect("Client initialization is valid")
        }

        async fn send_tx_to_da(
            &self,
            raw: Vec<u8>,
        ) -> Result<serde_json::Value, jsonrpsee::core::Error> {
            let blob = vec![raw].try_to_vec()?;
            let client = self.make_client();
            let fee: u64 = 2000;
            let namespace = ROLLUP_NAMESPACE_RAW.to_vec();
            let gas_limit = (blob.len() + 512) * GAS_PER_BYTE + 1060;

            let mut params = ArrayParams::new();
            params.insert(namespace)?;
            params.insert(blob)?;
            params.insert(fee.to_string())?;
            params.insert(gas_limit)?;
            client
                .request::<serde_json::Value, _>("state.SubmitPayForBlob", params)
                .await
        }
    }

    fn register_rpc_methods(rpc: &mut RpcModule<Ethereum>) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method(
            "eth_sendRawTransaction",
            |parameters, ethereum| async move {
                let data: Bytes = parameters.one().unwrap();

                let raw_evm_tx = RawEvmTransaction { tx: data.to_vec() };
                let (tx_hash, raw_tx) = ethereum
                    .make_raw_tx(raw_evm_tx)
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                ethereum
                    .send_tx_to_da(raw_tx)
                    .await
                    .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

                Ok::<_, ErrorObjectOwned>(tx_hash)
            },
        )?;

        Ok(())
    }

    fn default_max_response_size() -> u32 {
        1024 * 1024 * 100 // 100 MB
    }
}
