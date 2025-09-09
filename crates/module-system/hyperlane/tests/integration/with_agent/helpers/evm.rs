use crate::with_agent::helpers::docker::print_logs_from_container;
use crate::with_agent::helpers::hyperlane_cli::HyperlaneCliRunner;
use crate::with_agent::helpers::{EVM_MAILBOX, RELAYER_ACCOUNT};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sov_hyperlane_integration::{EthAddress, Message};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, HexHash, HexString};
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::anvil::AnvilNode;

pub const ANVIL_PORT: u16 = 8545;
const TAG: &str = "v1.1.0";

pub struct AnvilRunner {
    container: ContainerAsync<AnvilNode>,
    req_id: u64,
    host_port: u16,
}

impl AnvilRunner {
    pub async fn new() -> Self {
        tracing::info!("Starting anvil container...");
        // Hard code tag, so we don't accidental breakages
        let container = AnvilNode::default()
            .with_tag(TAG)
            .with_cmd(["--port", &ANVIL_PORT.to_string()])
            .start()
            .await
            .expect("failed to start anvil");

        let host_port = container
            .get_host_port_ipv4(ANVIL_PORT)
            .await
            .expect("Failed to get anvil port");
        tracing::info!(container_id = ?container.id(), %host_port, "Anvil container started successfully");

        Self {
            container,
            req_id: 0,
            host_port,
        }
    }

    pub fn port(&self) -> u16 {
        self.host_port
    }

    // Cast call is used to modify state
    pub async fn cast_call(
        &self,
        contract: EthAddress,
        abi: &str,
        args: impl AsRef<[&str]>,
        value: Amount,
    ) -> Vec<EvmLog> {
        let contract = contract.to_string();
        let value = value.to_string();
        let command = [
            &["cast", "send", contract.as_str(), abi][..],
            args.as_ref(),
            &[
                "--value",
                value.as_str(),
                "--private-key",
                RELAYER_ACCOUNT.1,
                "--json",
            ][..],
        ]
        .concat();

        tracing::info!(?command, container_id = ?self.container.id(), "executing cast call");

        let mut result = self
            .container
            .exec(ExecCommand::new(command.clone()))
            .await
            .unwrap();

        let mut exit_code = result.exit_code().await.expect("Failed to get exit code");
        for _ in 0..300 {
            exit_code = result.exit_code().await.expect("Failed to get exit code");
            if exit_code.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        tracing::info!(?command, ?exit_code, "executed cast call");
        let output = result.stdout_to_vec().await.unwrap();
        if exit_code != Some(0) {
            let std_err = result.stderr_to_vec().await.unwrap();
            panic!(
                "Failed to cast call.\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output),
                String::from_utf8_lossy(&std_err),
            );
        }

        let output: CallOutput = serde_json::from_slice(&output).unwrap();

        output.logs
    }

    // RPC is used to query data.
    pub async fn rpc<T: DeserializeOwned>(&mut self, method: &str, params: Value) -> T {
        let start = std::time::Instant::now();
        let port = self.host_port;
        let req_id = self.req_id.checked_add(1).unwrap();
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
        let hyperlane_deploy_start = std::time::Instant::now();
        let evm_recipient = hyperlane_cli.deploy_core().await;
        tracing::info!(time = ?hyperlane_deploy_start.elapsed(), "Hyperlane deployed");
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

        // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/main/solidity/contracts/Mailbox.sol#L110
        let logs = self
            .anvil
            .cast_call(
                EVM_MAILBOX,
                "dispatch(uint32,bytes32,bytes)",
                [
                    // destination domain
                    dest_domain.to_string().as_str(),
                    // recipient
                    recipient.to_string().as_str(),
                    // message
                    HexString(b"hello world".to_vec()).to_string().as_str(),
                ],
                Amount(0),
            )
            .await;
        EvmDispatchWithId::new(logs)
    }

    pub async fn deploy_warp_route(&mut self, sovtest_route: HexHash) -> HexHash {
        let ethtest_route_id = self.hyperlane_cli.deploy_warp().await;
        tracing::debug!(%ethtest_route_id, "Route deployed on anvil, enrolling");

        let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
        self.anvil
            .cast_call(
                hex_hash_into_eth_addr(&ethtest_route_id),
                "enrollRemoteRouter(uint32,bytes32)",
                [
                    domain.to_string().as_str(),
                    sovtest_route.to_string().as_str(),
                ],
                Amount(0),
            )
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
        let destination = config_value!("HYPERLANE_BRIDGE_DOMAIN").to_string();

        // https://github.com/hyperlane-xyz/hyperlane-monorepo/tree/c177c4733de52f8a2477ad74b46b3f1eebb5740b/solidity/contracts/token/libs/TokenRouter.sol#L54
        let logs = self
            .anvil
            .cast_call(
                route_addr,
                "transferRemote(uint32,bytes32,uint256)",
                [
                    // destination domain
                    destination.as_str(),
                    // recipient
                    recipient.to_string().as_str(),
                    // amount
                    amount.to_string().as_str(),
                ],
                // we don't need to pay fees on counterparty
                // so we only need to give contract what we want to send
                amount,
            )
            .await;

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

    pub async fn latest_message(&mut self) -> EvmProcessWithId {
        // fetch logs in the latest block
        let logs: Vec<_> = self.anvil.rpc("eth_getLogs", json!([{}])).await;
        EvmProcessWithId::new(logs)
    }

    /// Returns (origin_domain, recipient)
    pub async fn latest_warp_transfer(&mut self, token_addr: HexHash) -> (u32, HexHash) {
        let token_eth_addr = hex_hash_into_eth_addr(&token_addr);
        let logs: Vec<EvmLog> = self.anvil.rpc("eth_getLogs", json!([{}])).await;
        let log = logs
            .into_iter()
            .find(|log| log.address.0 == token_eth_addr.0)
            .unwrap();

        // first topic is event signature
        assert_eq!(
            log.topics.len(),
            3,
            "wrong number of topic of warp transfer event"
        );

        let origin_domain = domain_from_hexhash(log.topics[1]);
        (origin_domain, log.topics[2])
    }
}

#[derive(Debug, Deserialize)]
struct CallOutput {
    logs: Vec<EvmLog>,
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
    pub fn new(logs: impl IntoIterator<Item = EvmLog>) -> Self {
        let mut logs = logs.into_iter().filter(|log| log.address == EVM_MAILBOX);
        let process = logs.next().expect("Didn't find first event: Process");
        let process_id = logs
            .next()
            .expect("Didn't find the second event: ProcessId");

        // we should only have 2 logs from the mailbox
        assert!(logs.next().is_none());

        // Fields on evm have the same order as our events
        assert_eq!(process.topics.len(), 4);
        assert_eq!(process_id.topics.len(), 2);

        EvmProcessWithId {
            origin_domain: domain_from_hexhash(process.topics[1]),
            sender_address: process.topics[2],
            recipient_address: process.topics[3],
            id: process_id.topics[1],
        }
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
