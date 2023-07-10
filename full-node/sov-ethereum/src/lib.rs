use borsh::ser::BorshSerialize;
use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::app::DefaultPrivateKey;
use demo_stf::runtime::{DefaultContext, Runtime};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::Bytes;
use ethers::utils::rlp::Rlp;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::http_client::{HeaderMap, HttpClient};
use jsonrpsee::RpcModule;
use jupiter::da_service::DaServiceConfig;
use sov_evm::call::CallMessage;
use sov_evm::evm::EvmTransaction;
use sov_modules_api::transaction::Transaction;

const GAS_PER_BYTE: usize = 120;

pub fn get_ethereum_rpc(config: DaServiceConfig) -> RpcModule<Ethereum> {
    let e = Ethereum { config };
    let mut rpc = RpcModule::new(e);
    register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
    rpc
}

pub struct Ethereum {
    config: DaServiceConfig,
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
        let blob = vec![raw].try_to_vec().unwrap();
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

            // todo handle panics and unwraps.
            if data[0] > 0x7f {
                panic!("Invalid transaction type")
            }

            let rlp = Rlp::new(data);
            let (decoded_tx, _decoded_sig) = TypedTransaction::decode_signed(&rlp).unwrap();
            let tx_hash = decoded_tx.sighash();

            let tx_request = match decoded_tx {
                TypedTransaction::Legacy(_) => panic!("Legacy transaction type not supported"),
                TypedTransaction::Eip2930(_) => panic!("Eip2930 not supported"),
                TypedTransaction::Eip1559(request) => request,
            };

            let evm_tx = EvmTransaction {
                caller: tx_request.from.unwrap().into(),
                data: tx_request.data.unwrap().to_vec(),
                // todo set `gas limit`
                gas_limit: u64::MAX,
                // todo set `gas price`
                gas_price: Default::default(),
                // todo set `max_priority_fee_per_gas`
                max_priority_fee_per_gas: Default::default(),
                // todo `set to`
                to: None,
                value: tx_request.value.unwrap().into(),
                nonce: tx_request.nonce.unwrap().as_u64(),
                access_lists: vec![],
            };

            // todo set nonce
            let raw = make_raw_tx(evm_tx, 0).unwrap();
            ethereum.send_tx_to_da(raw).await?;

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
