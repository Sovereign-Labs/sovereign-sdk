use std::net::SocketAddr;
use std::time::Duration;

use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, TxKind, U256};
use sov_address::{EthereumAddress, MultiAddress};
use sov_evm::{AccountData, ContractCreationPolicy, EvmChainSpec, EvmGenesisConfig, SpecId};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::macros::config_value;
use sov_modules_api::Amount;
use sov_modules_stf_blueprint::GenesisParams;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath, TestRollup};

use crate::runtime::EvmBlueprint;

const SENDER_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const RECEIPT_POLL_TIMEOUT: Duration = Duration::from_secs(60);

async fn rpc_call(
    client: &reqwest::Client,
    addr: SocketAddr,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    client
        .post(format!("http://{addr}/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_eth_send_raw_transaction() {
    sov_test_utils::initialize_logging();
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();

    let signer: PrivateKeySigner = SENDER_KEY.parse().unwrap();
    let sender_eth_addr = signer.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData::empty_with_address(sender_eth_addr)],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
    };

    let mut genesis =
        crate::runtime::GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    genesis
        .bank
        .gas_token_config
        .as_mut()
        .expect("test requires a gas token config to fund the EVM sender")
        .address_and_balances
        .push((
            MultiAddress::Vm(EthereumAddress::from(sender_eth_addr)),
            Amount::new(100_000_000_000_000_000),
        ));

    let seq_da_address = genesis.sequencer_registry.sequencer_config.seq_da_address;

    // Shared DA layer so blobs submitted by the sequencer are visible to the block producer.
    let da_layer = std::sync::Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(1).await.unwrap(),
    ));
    let test_rollup: TestRollup<EvmBlueprint> = RollupBuilder::<EvmBlueprint>::new(
        GenesisSource::CustomParams(GenesisParams { runtime: genesis }),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        1,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir.into());
        c.rollup_prover_config = None;
    })
    .set_da_config(|c| {
        c.sender_address = seq_da_address;
        c.da_layer = Some(da_layer.clone());
    })
    .start()
    .await
    .unwrap();

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;

    let addr = test_rollup.http_addr;

    // eth_chainId is served by the Evm module (requires expose_rpc on the runtime);
    // eth_sendRawTransaction is served by sov_ethereum::get_ethereum_rpc via AdditionalSequencerApis.
    let chain_id: u64 = config_value!("CHAIN_ID");

    let recipient = Address::repeat_byte(0x42);
    let tx = TxEip1559 {
        chain_id,
        nonce: 0,
        gas_limit: 1_000_000,
        to: TxKind::Call(recipient),
        value: U256::ZERO,
        max_fee_per_gas: 100,
        ..Default::default()
    };
    let sig = signer.sign_hash(&tx.signature_hash()).await.unwrap();
    let signed = TxEnvelope::Eip1559(tx.into_signed(sig));
    let raw_tx_hex = hex::encode_prefixed(signed.encoded_2718());

    let resp = rpc_call(
        &client,
        addr,
        "eth_sendRawTransaction",
        serde_json::json!([raw_tx_hex]),
    )
    .await;
    let tx_hash = resp["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_sendRawTransaction failed: {resp}"));

    test_rollup.force_close_batch().await.unwrap();

    let receipt = tokio::time::timeout(RECEIPT_POLL_TIMEOUT, async {
        loop {
            let h = test_rollup.height().await.get();
            test_rollup.wait_for_height(h + 1).await;
            let resp = rpc_call(
                &client,
                addr,
                "eth_getTransactionReceipt",
                serde_json::json!([tx_hash]),
            )
            .await;
            assert!(
                resp.get("error").is_none(),
                "eth_getTransactionReceipt returned an error: {}",
                resp["error"]
            );
            if !resp["result"].is_null() {
                return resp["result"].clone();
            }
        }
    })
    .await
    .expect("timed out waiting for transaction receipt");

    assert_eq!(
        receipt["status"].as_str().unwrap(),
        "0x1",
        "transaction should succeed"
    );
    assert_eq!(receipt["transactionHash"].as_str().unwrap(), tx_hash);

    test_rollup.shutdown().await.unwrap();
}
