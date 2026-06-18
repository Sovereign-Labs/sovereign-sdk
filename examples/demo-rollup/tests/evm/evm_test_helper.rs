//! Minimal helpers for the two EVM smoke tests that remain in demo-rollup
//! (`evm_rpc.rs` and `evm_paymaster_balance_check.rs`). The full EVM test suite
//! and its associated helpers now live in `crates/full-node/sov-ethereum/tests/`.

use std::net::SocketAddr;

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, Bytes, TxKind, B256, U256, U64};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use reqwest::Url;
use serde::Serialize;
use sov_demo_rollup::{MockDemoRollup, MockRollupSpec};
use sov_eth_client::SimpleStorageClient;
use sov_evm_test_utils::LegacySimpleStorage;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_sequencer::{SeqConfigExtension, SovRateLimiterConfig};
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};

use crate::test_helpers::test_genesis_source;

pub(crate) const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

pub(crate) const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30),
};
pub(crate) const MAX_FEE_PER_GAS: u128 = 1_000_000_000;

async fn start_node(
    finalization_blocks: u32,
    extension: Option<SeqConfigExtension>,
    rate_limiter: Option<SovRateLimiterConfig<<MockRollupSpec<Native> as Spec>::Address>>,
    ideal_lag: u64,
) -> TestRollup<MockDemoRollup<Native>> {
    RollupBuilder::new(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        finalization_blocks,
    )
    .with_rate_limiter(rate_limiter)
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 65536;
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

pub(crate) async fn create_simple_storage_client(
    rest_port: SocketAddr,
    private_key: &str,
) -> SimpleStorageClient {
    let contract = LegacySimpleStorage::default();
    SimpleStorageClient::new(private_key, contract, rest_port).await
}

pub(crate) fn alloy_client(socket: SocketAddr) -> DynProvider {
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let url = Url::parse(&format!("http://{socket}/rpc")).unwrap();
    ProviderBuilder::new()
        .wallet(signer)
        .connect_http(url)
        .erased()
}

pub(crate) async fn deploy_contract_check(
    client: &SimpleStorageClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    let runtime_code = client.deploy_contract_call().await?;
    let tx_hash = client.deploy_contract().await?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    let contract_address = receipt.contract_address.unwrap();
    let code = client.eth_get_code(contract_address).await;
    assert_eq!(code[..runtime_code.len()], runtime_code.to_vec());
    Ok(contract_address)
}

pub(crate) async fn set_value_check(
    client: &SimpleStorageClient,
    contract_address: Address,
    set_arg: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx_hash = client.set_value(contract_address, set_arg).await;
    client.wait_for_receipt(tx_hash).await;
    let get_arg = client.query_contract(contract_address).await?;
    assert_eq!(U256::from(set_arg), get_arg);
    let storage_value = client
        .eth_get_storage_at(contract_address, U256::from(0x0))
        .await;
    assert_eq!(storage_value, U256::from(set_arg));
    Ok(())
}

pub async fn setup_test_rollup(
    finalization_blocks: u32,
    extension: SeqConfigExtension,
) -> TestRollup<MockDemoRollup<Native>> {
    start_node(finalization_blocks, Some(extension), None, 3).await
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
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 65536;
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
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 65536;
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

#[derive(Debug)]
pub(crate) struct EndpointResults {
    pub estimate_gas: Result<U64, jsonrpsee::core::client::Error>,
    pub call: Result<String, jsonrpsee::core::client::Error>,
    pub create_access_list: Result<AccessListResult, String>,
    pub send_raw_tx: Result<B256, jsonrpsee::core::client::Error>,
}

pub(crate) async fn call_all_endpoints(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    signer: &PrivateKeySigner,
) -> EndpointResults {
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

    let create_access_list: Result<AccessListResult, String> = match access_list_result {
        Ok(alr) if alr.error.is_some() => Err(alr.error.unwrap()),
        Ok(alr) => Ok(alr),
        Err(e) => Err(e.to_string()),
    };

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
            let gas_price: U256 = client
                .ws
                .request("eth_gasPrice", rpc_params![])
                .await
                .unwrap();
            gas_price.to::<u128>()
        }
    };

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
