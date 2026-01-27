use crate::with_agent::helpers::hyperlane_cli::HyperlaneCliRunner;
use crate::with_agent::helpers::{
    eth_address_to_hexhash, EVM_MAILBOX, EVM_MERKLE_TREE_HOOK, EVM_TEST_RECIPIENT, RELAYER_ACCOUNT,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use sov_hyperlane_integration::{EthAddress, Message};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, HexHash, HexString};
use sov_test_utils::docker::print_logs_from_container;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use testcontainers::core::Mount;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::anvil::AnvilNode;

pub const ANVIL_PORT: u16 = 8545;
const TAG: &str = "v1.3.6";
const ANVIL_STATE_FILE: &str = "anvil_core_state.json";

pub struct AnvilRunner {
    container: ContainerAsync<AnvilNode>,
    req_id: AtomicU64,
    host_port: u16,
    loaded_state: bool,
}

impl AnvilRunner {
    pub async fn new() -> Self {
        tracing::info!("Starting anvil container...");
        // Hard code tag, so we don't accidental breakages
        let (container, loaded_state) = {
            let state_dir = fixtures_dir();
            let state_path = state_dir.join(ANVIL_STATE_FILE);
            let use_state = state_path.exists();

            let mut node = AnvilNode::default().with_tag(TAG);
            if use_state {
                let state_mount =
                    Mount::bind_mount(state_dir.to_string_lossy().to_string(), "/state");
                let load_path = format!("/state/{ANVIL_STATE_FILE}");
                node = node
                    .with_mount(state_mount)
                    .with_cmd(vec!["--load-state".to_string(), load_path]);
            }

            let container = node.start().await.expect("failed to start anvil");
            (container, use_state)
        };

        let host_port = container
            .get_host_port_ipv4(ANVIL_PORT)
            .await
            .expect("Failed to get anvil port");
        tracing::info!(container_id = ?container.id(), %host_port, "Anvil container started successfully");

        Self {
            container,
            req_id: AtomicU64::new(0),
            host_port,
            loaded_state,
        }
    }

    pub fn port(&self) -> u16 {
        self.host_port
    }

    pub fn loaded_state(&self) -> bool {
        self.loaded_state
    }

    /// Send a transaction directly via Anvil JSON-RPC (avoids parsing `cast` CLI output).
    /// Returns the logs from the mined receipt.
    pub async fn send_transaction(
        &self,
        contract: EthAddress,
        data: Vec<u8>,
        value: Amount,
    ) -> Vec<EvmLog> {
        let tx = json!({
            "from": RELAYER_ACCOUNT.0,
            "to": contract.to_string(),
            "data": format!("0x{}", hex::encode(data)),
            "value": format!("0x{:x}", value.0),
            // 5_000_000 gas — plenty for the simple calls we issue in tests.
            "gas": "0x4c4b40",
        });

        let tx_hash: String = self.rpc("eth_sendTransaction", json!([tx])).await;
        tracing::info!(%tx_hash, "submitted tx to anvil");

        // Poll for receipt with a bounded timeout to avoid hangs on CI.
        let start = Instant::now();
        loop {
            let receipt: Option<TransactionReceipt> = self
                .rpc("eth_getTransactionReceipt", json!([tx_hash]))
                .await;

            if let Some(receipt) = receipt {
                if let Some(status) = receipt.status.as_deref() {
                    if status != "0x1" {
                        panic!(
                            "Transaction {tx_hash} failed with status {status:?}. Full receipt: {receipt:?}"
                        );
                    }
                }
                return receipt.logs;
            }

            if start.elapsed() > Duration::from_secs(30) {
                panic!("Timed out waiting for receipt for tx {tx_hash}");
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // RPC is used to query data.
    pub async fn rpc<T: DeserializeOwned>(&self, method: &str, params: Value) -> T {
        let start = std::time::Instant::now();
        let port = self.host_port;
        let req_id = self
            .req_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let resp = reqwest::Client::new()
            // Here we call on localhost, because anvil exposes port to the host machine.
            .post(format!("http://127.0.0.1:{port}"))
            .json(&json!({
                "id": req_id,
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();

        if let Some(error) = resp.get("error") {
            panic!("Errors calling anvil json-rpc: {error:?}");
        }

        tracing::info!(%method, response = ?resp, time = ?start.elapsed(), "Anvil call response");
        serde_json::from_value(resp["result"].clone()).unwrap()
    }

    pub async fn print_logs(&self) {
        print_logs_from_container("anvil", &self.container).await;
    }

    pub async fn assert_preloaded_core(&self) {
        self.ensure_contract_code(EVM_MAILBOX, "mailbox").await;
        self.ensure_contract_code(EVM_MERKLE_TREE_HOOK, "merkleTreeHook")
            .await;
        self.ensure_contract_code(EVM_TEST_RECIPIENT, "testRecipient")
            .await;

        let mailbox_nonce: String = self
            .eth_call_hex(EVM_MAILBOX, encode_no_args("nonce()"))
            .await;
        let merkle_count: String = self
            .eth_call_hex(EVM_MERKLE_TREE_HOOK, encode_no_args("count()"))
            .await;

        if !is_zero_hex(&mailbox_nonce) || !is_zero_hex(&merkle_count) {
            panic!(
                "Expected preloaded core to have zero mailbox nonce and merkle tree count; got nonce={mailbox_nonce}, count={merkle_count}"
            );
        }
    }

    async fn eth_call_hex(&self, contract: EthAddress, data: Vec<u8>) -> String {
        let call = json!({
            "to": contract.to_string(),
            "data": format!("0x{}", hex::encode(data)),
        });
        self.rpc("eth_call", json!([call, "latest"])).await
    }

    async fn ensure_contract_code(&self, contract: EthAddress, name: &str) {
        let code: String = self
            .rpc("eth_getCode", json!([contract.to_string(), "latest"]))
            .await;
        if code == "0x" {
            panic!("Expected {name} to be deployed in preloaded state");
        }
    }
}

pub struct EvmCounterParty {
    pub anvil: AnvilRunner,
    pub hyperlane_cli: HyperlaneCliRunner,
    pub evm_recipient: HexHash,
}

impl EvmCounterParty {
    pub async fn new(rollup_port: u16, host_address: &str) -> Self {
        let anvil = AnvilRunner::new().await;
        let anvil_port = anvil.port();
        let hyperlane_cli = HyperlaneCliRunner::new(rollup_port, anvil_port, host_address);
        let evm_recipient = if anvil.loaded_state() {
            tracing::info!("Using preloaded Anvil state for Hyperlane core");
            anvil.assert_preloaded_core().await;
            eth_address_to_hexhash(EVM_TEST_RECIPIENT)
        } else {
            let hyperlane_deploy_start = std::time::Instant::now();
            let evm_recipient = hyperlane_cli.deploy_core().await;
            tracing::info!(time = ?hyperlane_deploy_start.elapsed(), "Hyperlane deployed");
            evm_recipient
        };
        Self {
            anvil,
            hyperlane_cli,
            evm_recipient,
        }
    }

    pub async fn print_logs(&self) {
        self.anvil.print_logs().await;
    }

    pub async fn mine_block(&mut self) {
        self.anvil.rpc::<Value>("anvil_mine", json!([1])).await;
    }

    pub async fn dispatch_msg_to(&self, recipient: HexHash) -> EvmDispatchWithId {
        let dest_domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

        let data = encode_dispatch(
            dest_domain,
            recipient,
            HexString(b"hello world".to_vec()).as_ref(),
        );
        let logs = self
            .anvil
            .send_transaction(EVM_MAILBOX, data, Amount(0))
            .await;
        EvmDispatchWithId::new(logs)
    }

    pub async fn deploy_warp_route(&mut self, sovtest_route: HexHash) -> HexHash {
        let ethtest_route_id = self.hyperlane_cli.deploy_warp().await;
        tracing::debug!(%ethtest_route_id, "Route deployed on anvil, enrolling");

        let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
        let data = encode_enroll_remote_router(domain, sovtest_route);
        self.anvil
            .send_transaction(hex_hash_into_eth_addr(&ethtest_route_id), data, Amount(0))
            .await;

        ethtest_route_id
    }

    pub async fn send_warp_token_transfer(
        &mut self,
        ethtest_route_id: HexHash,
        recipient: HexHash,
        amount: Amount,
    ) -> EvmDispatchWithId {
        let route_addr = HexString::new(ethtest_route_id.0[12..].try_into().unwrap());
        let destination = config_value!("HYPERLANE_BRIDGE_DOMAIN");

        // https://github.com/hyperlane-xyz/hyperlane-monorepo/tree/c177c4733de52f8a2477ad74b46b3f1eebb5740b/solidity/contracts/token/libs/TokenRouter.sol#L54
        let data = encode_transfer_remote(destination, recipient, amount);
        // we don't need to pay fees on counterparty
        // so we only need to give contract what we want to send
        let logs = self.anvil.send_transaction(route_addr, data, amount).await;

        EvmDispatchWithId::new(logs)
    }

    pub async fn balance_of(&mut self, address: HexHash) -> Amount {
        let addr = hex_hash_into_eth_addr(&address);
        let mut balance: String = self
            .anvil
            .rpc("eth_getBalance", json!([addr.to_string(), "latest"]))
            .await;

        // evm can encode first byte in a single hex character if it fits
        // but `hex::decode` expects each byte to be encoded in two characters
        // so if this is a case, we 0-prefix it after '0x' prefix
        if balance.len() % 2 == 1 {
            balance.insert(2, '0');
        }
        let balance: HexString = balance.parse().unwrap();

        let mut amount = [0; 16];
        amount[16 - balance.0.len()..].copy_from_slice(&balance.0);

        Amount(u128::from_be_bytes(amount))
    }

    pub async fn latest_message(&mut self) -> Result<EvmProcessWithId, String> {
        // fetch logs in the latest block
        let logs: Vec<_> = self.anvil.rpc("eth_getLogs", json!([{}])).await;
        EvmProcessWithId::try_new(logs)
    }

    /// Returns (origin_domain, recipient)
    pub async fn latest_warp_transfer(&mut self, token_addr: HexHash) -> (u32, HexHash) {
        let token_eth_addr = hex_hash_into_eth_addr(&token_addr);
        let start = Instant::now();

        loop {
            let logs: Vec<EvmLog> = self.anvil.rpc("eth_getLogs", json!([{}])).await;
            if let Some(log) = logs
                .into_iter()
                .find(|log| log.address.0 == token_eth_addr.0)
            {
                // first topic is event signature
                assert_eq!(
                    log.topics.len(),
                    3,
                    "wrong number of topic of warp transfer event"
                );

                let origin_domain = domain_from_hexhash(log.topics[1]);
                let recipient = log.topics[2];
                return (origin_domain, recipient);
            }

            if start.elapsed() > Duration::from_secs(30) {
                panic!(
                    "Timed out waiting for warp transfer log for token {token_addr}. \
                     Consider checking relayer/anvil logs."
                );
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

#[derive(Debug, Deserialize)]
struct TransactionReceipt {
    logs: Vec<EvmLog>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EvmLog {
    address: EthAddress,
    /// The first topic is keccak hash of the event's signature
    /// followed by indexed event's fields in order they are defined.
    topics: Vec<HexHash>,
    /// Data holds abi encoded non-indexed event's fields
    data: HexString,
}

pub struct EvmProcessWithId {
    /// The origin domain of the message.
    pub origin_domain: u32,
    /// The sender address of the message.
    pub sender_address: HexHash,
    /// The recipient address of the message.
    pub recipient_address: HexHash,
    /// The ID of the message.
    pub id: HexHash,
}

impl EvmProcessWithId {
    /// Reconstruct combined process event from mailbox logs.
    /// https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/interfaces/IMailbox.sol#L29-L45
    pub fn try_new(logs: impl IntoIterator<Item = EvmLog>) -> Result<Self, String> {
        let mut logs = logs.into_iter().filter(|log| log.address == EVM_MAILBOX);
        let process = logs
            .next()
            .ok_or_else(|| "Didn't find first event: Process".to_string())?;
        let process_id = logs
            .next()
            .ok_or_else(|| "Didn't find the second event: ProcessId".to_string())?;

        // we should only have 2 logs from the mailbox
        if logs.next().is_some() {
            return Err("Expected only 2 mailbox logs for Process/ProcessId".to_string());
        }

        // Fields on evm have the same order as our events
        if process.topics.len() != 4 {
            return Err(format!(
                "Unexpected Process topics count: {}",
                process.topics.len()
            ));
        }
        if process_id.topics.len() != 2 {
            return Err(format!(
                "Unexpected ProcessId topics count: {}",
                process_id.topics.len()
            ));
        }

        Ok(EvmProcessWithId {
            origin_domain: domain_from_hexhash(process.topics[1]),
            sender_address: process.topics[2],
            recipient_address: process.topics[3],
            id: process_id.topics[1],
        })
    }
}

#[derive(Debug)]
pub struct EvmDispatchWithId {
    /// The sender address of the message.
    pub sender_address: HexHash,
    /// The destination domain of the message.
    pub destination_domain: u32,
    /// The recipient address of the message.
    pub recipient_address: HexHash,
    /// The message that was dispatched.
    pub message: Message,
    /// The ID of the message.
    pub message_id: HexHash,
}

impl EvmDispatchWithId {
    /// Reconstruct combined dispatch event from mailbox logs.
    /// https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/interfaces/IMailbox.sol#L9-L27
    fn new(logs: impl IntoIterator<Item = EvmLog>) -> Self {
        let mut logs = logs.into_iter().filter(|log| log.address == EVM_MAILBOX);
        let dispatch = logs.next().unwrap();
        let dispatch_id = logs.next().unwrap();

        // we should only have 2 logs from the mailbox
        assert!(logs.next().is_none());

        // Fields on evm have the same order as our events
        assert_eq!(dispatch.topics.len(), 4);
        assert_eq!(dispatch_id.topics.len(), 2);

        // first 32 bytes is field's offset, always 0x20 for the first field
        // next 32 bytes is the length of the field bytes
        let encoded_len = &dispatch.data.0[32..64];
        assert!(encoded_len.iter().take(28).all(|&byte| byte == 0));
        let message_len = u32::from_be_bytes(encoded_len[28..].try_into().unwrap());
        // next comes the field's data, with the length we just parsed, padded with 0' to the
        // multiplier of 32
        let message_bytes = &dispatch.data.0[64..64 + message_len as usize];

        EvmDispatchWithId {
            sender_address: dispatch.topics[1],
            destination_domain: domain_from_hexhash(dispatch.topics[2]),
            recipient_address: dispatch.topics[3],
            message: Message::decode(message_bytes).unwrap(),
            message_id: dispatch_id.topics[1],
        }
    }
}

fn domain_from_hexhash(hash: HexHash) -> u32 {
    assert!(hash.0[0..28].iter().all(|&b| b == 0));
    u32::from_be_bytes(hash.0[28..].try_into().unwrap())
}

fn hex_hash_into_eth_addr(hex_hash: &HexHash) -> EthAddress {
    let mut res = [0; 20];
    res[..].copy_from_slice(&hex_hash.0[12..]);
    res.into()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/integration/with_agent/fixtures")
}

fn function_selector(signature: &str) -> [u8; 4] {
    let mut hasher = Keccak256::new();
    hasher.update(signature.as_bytes());
    let hash = hasher.finalize();
    hash[..4].try_into().unwrap()
}

fn pad_u32(value: u32) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[28..].copy_from_slice(&value.to_be_bytes());
    buf
}

fn pad_u128(value: u128) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[16..].copy_from_slice(&value.to_be_bytes());
    buf
}

fn encode_no_args(signature: &str) -> Vec<u8> {
    function_selector(signature).to_vec()
}

fn is_zero_hex(value: &str) -> bool {
    value.trim_start_matches("0x").chars().all(|c| c == '0')
}

fn encode_dispatch(domain: u32, recipient: HexHash, body: &[u8]) -> Vec<u8> {
    // ABI encoding: selector || domain || recipient || offset || len || body || padding
    let selector = function_selector("dispatch(uint32,bytes32,bytes)");
    let mut data = Vec::with_capacity(4 + 32 * 3 + body.len() + 32);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&pad_u32(domain));
    data.extend_from_slice(&recipient.0);
    // offset to the start of the dynamic bytes section: 3 words = 0x60
    data.extend_from_slice(&pad_u128(96));
    data.extend_from_slice(&pad_u128(body.len() as u128));
    data.extend_from_slice(body);
    // pad body to 32-byte boundary
    while data.len() % 32 != 0 {
        data.push(0);
    }
    data
}

fn encode_enroll_remote_router(domain: u32, router: HexHash) -> Vec<u8> {
    let selector = function_selector("enrollRemoteRouter(uint32,bytes32)");
    let mut data = Vec::with_capacity(4 + 32 * 2);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&pad_u32(domain));
    data.extend_from_slice(&router.0);
    data
}

fn encode_transfer_remote(destination: u32, recipient: HexHash, amount: Amount) -> Vec<u8> {
    let selector = function_selector("transferRemote(uint32,bytes32,uint256)");
    let mut data = Vec::with_capacity(4 + 32 * 3);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&pad_u32(destination));
    data.extend_from_slice(&recipient.0);
    data.extend_from_slice(&pad_u128(amount.0));
    data
}
