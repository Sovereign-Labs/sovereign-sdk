#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_rpc, Ethereum};

#[cfg(feature = "experimental")]
pub mod experimental {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use anvil_core::eth::transaction::TypedTransaction;
    use borsh::ser::BorshSerialize;
    use const_rollup_config::ROLLUP_NAMESPACE_RAW;
    use demo_stf::app::DefaultPrivateKey;
    use demo_stf::runtime::{DefaultContext, Runtime};
    use ethers::types::Bytes;
    use ethers::utils::rlp;
    use jsonrpsee::core::client::ClientT;
    use jsonrpsee::core::params::ArrayParams;
    use jsonrpsee::http_client::{HeaderMap, HttpClient};
    use jsonrpsee::RpcModule;
    use jupiter::da_service::DaServiceConfig;
    use sov_evm::call::CallMessage;
    use sov_evm::evm::{EthAddress, EvmTransaction};
    use sov_modules_api::transaction::Transaction;

    const GAS_PER_BYTE: usize = 120;

    #[cfg(feature = "experimental")]
    pub fn get_ethereum_rpc(config: DaServiceConfig) -> RpcModule<Ethereum> {
        let mut rpc = RpcModule::new(Ethereum {
            config,
            nonces: Default::default(),
        });
        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct Ethereum {
        config: DaServiceConfig,
        nonces: Mutex<HashMap<EthAddress, u64>>,
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
                .max_request_body_size(default_max_response_size()) // 100 MB
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
                let data = data.as_ref();

                if data.is_empty() {
                    return Err(jsonrpsee::core::Error::Custom(
                        "Empty raw transaction data".to_owned(),
                    ));
                }

                if data[0] > 0x7f {
                    return Err(jsonrpsee::core::Error::Custom(
                        "Legacy transaction not supported".to_owned(),
                    ));
                }

                let extend = rlp::encode(&data);
                let typed_transaction = match rlp::decode::<TypedTransaction>(&extend[..]) {
                    Ok(transaction) => transaction,
                    Err(e) => {
                        return Err(jsonrpsee::core::Error::Custom(format!(
                            "Failed to decode signed transaction: {}",
                            e
                        )))
                    }
                };

                let transaction = match typed_transaction {
                    TypedTransaction::Legacy(_) => {
                        return Err(jsonrpsee::core::Error::Custom(
                            "Legacy transaction not supported".to_owned(),
                        ))
                    }
                    TypedTransaction::EIP2930(_) => {
                        return Err(jsonrpsee::core::Error::Custom(
                            "EIP2930 not supported".to_owned(),
                        ))
                    }
                    TypedTransaction::EIP1559(tx) => tx,
                };

                let tx_hash = transaction.hash();
                let evm_transaction: EvmTransaction = transaction.into();
                let sender = evm_transaction.sender;

                let raw_tx = {
                    let mut nonces = ethereum.nonces.lock().unwrap();
                    let n = nonces.entry(sender).and_modify(|n| *n += 1).or_insert(0);
                    make_raw_tx(evm_transaction, *n)?
                };

                ethereum.send_tx_to_da(raw_tx).await?;
                Ok(tx_hash)
            },
        )?;

        Ok(())
    }

    fn make_raw_tx(evm_tx: EvmTransaction, nonce: u64) -> Result<Vec<u8>, std::io::Error> {
        let tx = CallMessage { tx: evm_tx };
        let message = Runtime::<DefaultContext>::encode_evm_call(tx);
        // todo don't generate sender here.
        let sender = DefaultPrivateKey::generate();
        let tx = Transaction::<DefaultContext>::new_signed_tx(&sender, message, nonce);
        tx.try_to_vec()
    }

    fn default_max_response_size() -> u32 {
        1024 * 1024 * 100 // 100 MB
    }
}
