use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use crate::test_helpers::test_genesis_source;

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::DynProvider;
use alloy_provider::Provider as _;
use alloy_provider::ProviderBuilder;
use alloy_provider::WsConnect;
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Client;
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use sov_demo_rollup::MockRollupSpec;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::LegacySimpleStorage;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::SovRateLimiterConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{
    get_appropriate_rollup_prover_config, RollupBuilder, TestRollup,
};

pub(crate) const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub(crate) const SECONDARY_SENDER_PRIV_KEY: &str =
    "0x96eeea10d406ba7d4e74f7bb9e71b6378165162e4e42fd31c937f7728bbaa7b2";
/// Hardhat #1: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8
/// Not in any genesis → starts with zero EVM balance, not covered by selective paymaster.
pub(crate) const AFFORDABILITY_SIGNER_PRIV_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
/// Hardhat #4: 0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65
/// Not in any genesis → starts with zero EVM balance, covered by selective paymaster.
pub(crate) const PAYMASTER_SIGNER_PRIV_KEY: &str =
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";

pub(crate) const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30), // Limit our response size to 1MB, leaving 30kb for headers, overhead, and misestimation.
};
pub(crate) const MAX_FEE_PER_GAS: u128 = 1_000_000_000;
pub(crate) const HIGH_MAX_FEE_PER_GAS: u128 = 1_000_000_000_000;
pub(crate) const PAYER_SOV_BANK_BALANCE: u128 = 5_000_000_000_000_000;
pub(crate) const HIGH_PRIORITY_FEE_PER_GAS: u128 = 1;
pub(crate) const MAX_POLL_ATTEMPTS: usize = 100;
pub(crate) const POLL_INTERVAL_MS: u64 = 25;
pub(crate) const INSUFFICIENT_FUNDS_ERROR: &str = "insufficient funds for gas * price + value";
pub(crate) const FEE_CAP_TOO_LOW_ERROR: &str = "max fee per gas less than block base fee";

/// Starts test rollup node.  
pub(crate) async fn start_node(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
    rate_limiter: Option<SovRateLimiterConfig<<MockRollupSpec<Native> as Spec>::Address>>,
    ideal_lag: u64,
) -> TestRollup<MockDemoRollup<Native>> {
    // Don't provide a prover since the EVM is not currently provable
    RollupBuilder::new(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            // The lowest possible time is 1 second,
            // as subscription tests require new block to have increased timestamp in seconds.
            block_time_ms: 1_000,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .with_rate_limiter(rate_limiter)
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None; // FIXME(@neysofu): reenable once sov-ethereum is compatible with proof blobs
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = extension;
        if let sov_sequencer::SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
            seq.ideal_lag_behind_finalized_slot = ideal_lag;
        }
    })
    .start()
    .await
    .unwrap()
}

/// Creates a test simple storage client to communicate with the rollup node & SimpleStorage contract.
pub(crate) async fn create_simple_storage_client(
    rest_port: SocketAddr,
    private_key: &str,
) -> SimpleStorageClient {
    let contract = LegacySimpleStorage::default();
    SimpleStorageClient::new(private_key, contract, rest_port).await
}

pub(crate) async fn alloy_ws_client(socket: SocketAddr) -> DynProvider {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let url = Url::parse(&format!("ws://{socket}/rpc")).unwrap();
    let ws = WsConnect::new(url);
    ProviderBuilder::new()
        .wallet(signer)
        .connect_ws(ws)
        .await
        .unwrap()
        .erased()
}

pub(crate) fn alloy_client_with_signer(socket: SocketAddr, private_key: &str) -> DynProvider {
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased()
}

pub(crate) fn alloy_client(socket: SocketAddr) -> DynProvider {
    alloy_client_with_signer(socket, SENDER_PRIV_KEY)
}

pub(crate) async fn rpc_call(
    client: &Client,
    http_addr: SocketAddr,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    Ok(client
        .post(format!("http://{http_addr}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .send()
        .await?
        .json::<Value>()
        .await?)
}

pub(crate) fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

pub(crate) fn hex_word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}

pub(crate) fn hex_u128(value: u128) -> String {
    format!("0x{value:x}")
}

pub(crate) fn parse_hex_u64(value: &str) -> u64 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u64::from_str_radix(hex, 16).expect("valid u64 hex quantity")
}

pub(crate) fn parse_hex_u128(value: &str) -> u128 {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return 0;
    }
    u128::from_str_radix(hex, 16).expect("valid u128 hex quantity")
}

pub(crate) fn rpc_result_hex(response: &Value) -> String {
    response
        .get("result")
        .and_then(Value::as_str)
        .expect("result should be a hex string")
        .to_string()
}

pub(crate) fn rpc_error_code_from_response(response: &Value, method: &str) -> i64 {
    rpc_error_code(rpc_error_object(response, method))
}

pub(crate) async fn tx_count(
    client: &SimpleStorageClient,
    address: Address,
    block: impl Serialize,
) -> anyhow::Result<u64> {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await?;
    Ok(count.to::<u64>())
}

pub(crate) async fn raw_signed_eip1559(
    signer: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    to: TxKind,
    value: U256,
    input: Bytes,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
) -> anyhow::Result<String> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        to,
        value,
        input,
        access_list: Default::default(),
    };
    let sig = signer.sign_hash(&tx.signature_hash()).await?;
    let envelope = TxEnvelope::Eip1559(tx.into_signed(sig));
    Ok(format!("0x{}", hex::encode(envelope.encoded_2718())))
}

pub(crate) fn eth_call_params(from: &str, to: &str, input: &str, block_tag: &str) -> Value {
    json!([{
        "from": from,
        "to": to,
        "gas": "0x7a120",
        "input": input
    }, block_tag])
}

pub(crate) fn rpc_result_str<'a>(response: &'a Value, method: &str) -> &'a str {
    response
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{method} should return result"))
}

pub(crate) fn rpc_error_object<'a>(response: &'a Value, method: &str) -> &'a Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("{method} should return an error object"))
}

pub(crate) fn rpc_error_code(error: &Value) -> i64 {
    error
        .get("code")
        .and_then(Value::as_i64)
        .expect("error.code should be present")
}

pub(crate) fn rpc_error_message(error: &Value) -> &str {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(crate) fn rpc_error_data_str(error: &Value) -> Option<&str> {
    error.get("data").and_then(Value::as_str)
}

pub(crate) fn hash_selector(hash: B256) -> Value {
    json!({
        "blockHash": format!("{:#x}", hash),
        "requireCanonical": true
    })
}

pub(crate) async fn poll_until<T, F, Fut, P>(
    mut fetch: F,
    mut predicate: P,
    failure_msg: &str,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
    P: FnMut(&T) -> bool,
{
    let mut value = fetch().await?;
    for _ in 0..MAX_POLL_ATTEMPTS {
        if predicate(&value) {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        value = fetch().await?;
    }

    anyhow::bail!("{failure_msg}")
}

pub(crate) fn assert_invalid_params(response: &Value) {
    let error = rpc_error_object(response, "assert_invalid_params");
    assert_eq!(
        rpc_error_code(error),
        jsonrpsee::types::error::INVALID_PARAMS_CODE as i64,
        "expected JSON-RPC invalid params code"
    );
}

/// Estimates gas for `tx`, asserts the sender can afford it, and sets the gas limit.
/// The affordability check includes `tx.value` in the ceiling.
pub(crate) async fn estimate_gas_and_check_affordability(
    client: &SimpleStorageClient,
    tx: &mut TransactionRequest,
    max_fee_per_gas: u128,
    sender_balance: U256,
) -> anyhow::Result<()> {
    tx.gas = None;
    let estimated_gas_limit = client.eth_estimate_gas(tx.clone()).await;
    let tx_value = tx.value.unwrap_or(U256::ZERO);
    let ceiling = U256::from(estimated_gas_limit)
        .checked_mul(U256::from(max_fee_per_gas))
        .and_then(|cost| cost.checked_add(tx_value))
        .ok_or_else(|| anyhow::anyhow!("gas affordability ceiling overflow"))?;
    assert!(
        ceiling < sender_balance,
        "test precondition failed: gas ceiling {ceiling} must be below sender balance {sender_balance}"
    );
    tx.gas = Some(estimated_gas_limit);
    Ok(())
}

pub(crate) async fn finalized_block_number_and_hash(client: &SimpleStorageClient) -> (u64, B256) {
    let finalized_block = client
        .eth_get_block_by_number(Some("finalized".to_string()))
        .await;
    (finalized_block.header.number, finalized_block.header.hash)
}

pub(crate) fn alloy_client_with_reqwest<B>(
    socket: SocketAddr,
    b: B,
    private_key: &str,
) -> DynProvider
where
    B: FnOnce(reqwest::ClientBuilder) -> reqwest::Client,
{
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .with_reqwest(url, b)
        .erased()
}

/// Deploys a test contract on the test rollup.
pub(crate) async fn deploy_contract_check(
    client: &SimpleStorageClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    let runtime_code = client.deploy_contract_call().await?;

    let tx_hash = client.deploy_contract().await?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    let contract_address = receipt.contract_address.unwrap();

    // Assert contract deployed correctly
    let code = client.eth_get_code(contract_address).await;
    // code has natural following 0x00 bytes, so we need to trim it
    assert_eq!(code[..runtime_code.len()], runtime_code.to_vec());

    Ok(contract_address)
}

/// Calls `set_value` on the test contract.
pub(crate) async fn set_value_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hash = client.set_value(contract_address, set_arg).await;
    client.wait_for_receipt(tx_hash).await;

    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(U256::from(set_arg), get_arg);

    // Assert storage slot is set
    let storage_slot = 0x0;
    let storage_value = client
        .eth_get_storage_at(contract_address, U256::from(storage_slot))
        .await;
    assert_eq!(storage_value, U256::from(set_arg));

    Ok(())
}

/// Calls `set_values` on the test contract.
pub(crate) async fn set_multiple_values_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    values: Vec<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hashes = client.set_values(contract_address, values).await;

    // Wait for all receipts
    for tx_hash in tx_hashes {
        client.wait_for_receipt(tx_hash).await;
    }

    {
        let get_arg: u32 = client
            .query_contract(contract_address)
            .await?
            .try_into()
            .unwrap();
        // should be one of three values sent in a single block. 150, 151, or 152
        assert!((150..=152).contains(&get_arg));
    }

    Ok(())
}

pub async fn setup_test_rollup(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    setup_test_rollup_with_ideal_lag(finalization_blocks, extension, 3).await
}

pub async fn setup_test_rollup_with_ideal_lag(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    ideal_lag: u64,
) -> TestRollup<MockDemoRollup<Native>> {
    let host_args = mock_da_risc0_host_args();
    let config = get_appropriate_rollup_prover_config::<MockRollupSpec<Native>>(host_args);
    start_node(
        config,
        finalization_blocks,
        Some(extension),
        None,
        ideal_lag,
    )
    .await
}

pub async fn setup_test_rollup_with_paymaster(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    let mut paths = crate::test_helpers::test_genesis_paths(sov_modules_api::OperatingMode::Zk);
    paths.paymaster_genesis_path = std::path::PathBuf::from(
        "../test-data/genesis/integration-tests/paymaster_with_payer.json",
    );

    RollupBuilder::new(
        sov_test_utils::test_rollup::GenesisSource::Paths(paths),
        BlockProducingConfig::Periodic {
            block_time_ms: sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None;
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(extension);
        if let sov_sequencer::SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
            seq.ideal_lag_behind_finalized_slot = 1;
        }
    })
    .start()
    .await
    .unwrap()
}

pub async fn setup_test_rollup_with_selective_paymaster(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    let mut paths = crate::test_helpers::test_genesis_paths(sov_modules_api::OperatingMode::Zk);
    paths.paymaster_genesis_path =
        std::path::PathBuf::from("../test-data/genesis/integration-tests/paymaster_selective.json");

    RollupBuilder::new(
        sov_test_utils::test_rollup::GenesisSource::Paths(paths),
        BlockProducingConfig::Periodic {
            block_time_ms: sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None;
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(extension);
        if let sov_sequencer::SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
            seq.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .start()
    .await
    .unwrap()
}

#[derive(Debug)]
pub(crate) struct EndpointResults {
    pub estimate_gas: Result<U64, jsonrpsee::core::client::Error>,
    pub call: Result<String, jsonrpsee::core::client::Error>,
    pub create_access_list: Result<AccessListResult, String>,
    pub send_raw_tx: Result<B256, jsonrpsee::core::client::Error>,
}

/// Calls all 4 simulation/submission endpoints with the same request.
/// Returns individual results without any consistency assertions.
pub(crate) async fn call_all_endpoints(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    signer: &PrivateKeySigner,
) -> EndpointResults {
    // Step 1: 3 simulation calls
    let estimate_gas: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await;

    let call: Result<String, _> = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await;

    let access_list_result: Result<AccessListResult, _> = client
        .ws
        .request("eth_createAccessList", rpc_params![request, "latest"])
        .await;

    // Normalize eth_createAccessList: the RPC call itself may succeed but return
    // an error in the `error` field of the response.
    let create_access_list: Result<AccessListResult, String> = match access_list_result {
        Ok(alr) if alr.error.is_some() => Err(alr.error.unwrap()),
        Ok(alr) => Ok(alr),
        Err(e) => Err(e.to_string()),
    };

    // Step 2: Build and send raw tx
    let chain_id: U64 = client
        .ws
        .request("eth_chainId", rpc_params![])
        .await
        .unwrap();

    let nonce = match request.nonce {
        Some(n) => n,
        None => tx_count(client, signer.address(), "latest").await.unwrap(),
    };

    let max_fee = match request.gas_price.or(request.max_fee_per_gas) {
        Some(fee) => fee,
        None => {
            // Mirror simulation endpoints: when no fee is specified, use the current base fee.
            let gas_price: U256 = client
                .ws
                .request("eth_gasPrice", rpc_params![])
                .await
                .unwrap();
            gas_price.to::<u128>()
        }
    };

    // When gas is omitted, mirror what a real user would do: use the estimateGas result.
    let gas_limit = match request.gas {
        Some(g) => g,
        None => match &estimate_gas {
            Ok(estimated) => estimated.to::<u64>(),
            Err(_) => 1_000_000,
        },
    };

    let raw_tx = raw_signed_eip1559(
        signer,
        chain_id.to::<u64>(),
        nonce,
        gas_limit,
        request.to.unwrap_or(TxKind::Create),
        request.value.unwrap_or(U256::ZERO),
        request.input.input.clone().unwrap_or_default(),
        max_fee,
        request.max_priority_fee_per_gas.unwrap_or(0),
    )
    .await
    .unwrap();

    let send_raw_tx: Result<B256, _> = client
        .ws
        .request("eth_sendRawTransaction", rpc_params![&raw_tx])
        .await;

    EndpointResults {
        estimate_gas,
        call,
        create_access_list,
        send_raw_tx,
    }
}

pub async fn setup_with_simple_storage(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient, u64) {
    setup_with_simple_storage_with_ideal_lag(finalization_blocks, extension, 3).await
}

pub async fn setup_with_simple_storage_with_ideal_lag(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
    ideal_lag: u64,
) -> (TestRollup<MockDemoRollup<Native>>, SimpleStorageClient, u64) {
    let test_rollup =
        setup_test_rollup_with_ideal_lag(finalization_blocks, extension, ideal_lag).await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;
    let simple_storage = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;
    (test_rollup, simple_storage, config_value!("CHAIN_ID"))
}
