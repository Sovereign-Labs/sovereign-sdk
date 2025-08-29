mod evm;
mod hyperlane_cli;

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use super::configs::agent_config;
use super::preferred_sequencer_runtime::{GenesisConfig, TestRuntime};
use crate::with_agent::helpers::evm::AnvilRunner;
use crate::with_agent::helpers::hyperlane_cli::HyperlaneCliRunner;
use futures::future::join_all;
use futures::{FutureExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sov_bank::Amount;
use sov_hyperlane_integration::{EthAddress, Message};
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::macros::config_value;
use sov_modules_api::{CryptoSpec, HexHash, HexString, Spec};
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{RtAgnosticBlueprint, TestProver, TestSequencer, TestSpec, TestUser};
use testcontainers::core::client::docker_client_instance;
use testcontainers::core::{CmdWaitFor, ExecCommand, ExecResult, Host, IntoContainerPort};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::io::AsyncBufReadExt;
use tokio::time::timeout;

pub type RollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;
pub type TestRollupBuilder = RollupBuilder<RollupBlueprint, PathBuf>;
pub type PrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

type Container = ContainerAsync<GenericImage>;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 400;
pub const FINALIZED_BLOCKS_AT_START: usize = 3;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};
pub const ANVIL_PORT: u16 = 8545;
pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 10;
/// Use `container.get_host_port_ipv4(RELAYER_METRICS_PORT)` to get metrics
pub const RELAYER_METRICS_PORT: u16 = 9091;
pub const VALIDATOR_METRICS_PORT: u16 = 9097;
/// Domain id of the evm counterparty chain
/// Should match K
pub const EVM_DOMAIN: u32 = 31337_90210;
pub const EVM_CHAIN_ID: u32 = 31337;
/// Address of the mailbox on evm counterparty chain
/// 0x8A791620dd6260079BF849Dc5567aDC3F2FdC318
pub const EVM_MAILBOX: EthAddress = HexString([
    138, 121, 22, 32, 221, 98, 96, 7, 155, 248, 73, 220, 85, 103, 173, 195, 242, 253, 195, 24,
]);
/// Fixed Eth keys created by anvil. They don't change. Each address is funded 1000ETH
// run `docker run --rm ghcr.io/eigerco/hyperlane anvil` to see all keys
pub const ANVIL_ACCOUNTS: &[(&str, &str)] = &[
    (
        // First account is used by relayer, the rest belongs to validators
        "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    ),
    (
        "0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
    ),
    (
        "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc",
        "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a",
    ),
    (
        "0x90f79bf6eb2c4f870365e785982e1f101e93b906",
        "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
    ),
    (
        "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65",
        "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a",
    ),
    (
        "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc",
        "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba",
    ),
    (
        "0x976ea74026e726554db657fa54763abd0c3a0aa9",
        "0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e",
    ),
    (
        "0x14dc79964da2c08b23698b3d3cc7ca32193d9955",
        "0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356",
    ),
    (
        "0x23618e81e3f5cdf7f54c3d65f7fbc0abf5b21e8f",
        "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
    ),
    (
        "0xa0ee7a142d267c1f36714e4a8f75612f20a79720",
        "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6",
    ),
];

pub const RELAYER_ACCOUNT: (&str, &str) = ANVIL_ACCOUNTS[0];

pub struct Setup {
    pub sequencer: TestSequencer<TestSpec>,
    pub relayer: TestUser<TestSpec>,
    pub validators: Vec<TestUser<TestSpec>>,
    pub prover: TestProver<TestSpec>,
    pub genesis_config: GenesisConfig<TestSpec>,
}

pub fn generate_setup() -> Setup {
    let genesis_config =
        HighLevelZkGenesisConfig::generate_with_additional_accounts(ANVIL_ACCOUNTS.len());

    let relayer = genesis_config.additional_accounts()[0].clone();
    let validators = (1..ANVIL_ACCOUNTS.len())
        .map(|n| genesis_config.additional_accounts()[n].clone())
        .collect();
    let sequencer = genesis_config.initial_sequencer.clone();
    let prover = genesis_config.initial_prover.clone();

    let genesis_config =
        GenesisConfig::from_minimal_config(genesis_config.into(), (), (), (), (), (), ());

    Setup {
        sequencer,
        relayer,
        validators,
        prover,
        genesis_config,
    }
}

pub async fn setup_rollup(
    storage_path: PathBuf,
    setup: Setup,
    wait_for_finalized_slot: bool,
) -> TestRollup<RollupBlueprint, PathBuf> {
    let axum_bind_ip = if cfg!(target_os = "macos") {
        // MacOS runs docker inside the VM, so returned gateway IP does not match any address on the host.
        // Test containers already expose all the ports to `0.0.0.0` so this does not increase security risk significantly.
        // If better solution exists, happy to apply it
        "0.0.0.0".to_string()
    } else {
        get_docker_gateway_ip().await
    };
    let rollup_builder = TestRollupBuilder::new_with_storage_path(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        DEFAULT_BLOCK_PRODUCING_CONFIG,
        DEFAULT_FINALIZATION_BLOCKS,
        storage_path,
        true,
    )
    .set_config(|config| {
        config.rollup_prover_config = None;
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            ..Default::default()
        });
        config.prover_address = setup.prover.user_info.address().to_string();
        config.aggregated_proof_block_jump = 3;
        // Make rollup listen on docker host interface, so it can be accessed from containers.
        config.axum_host = axum_bind_ip;
        config.blob_processing_timeout_secs = 300;
    })
    .set_da_config(|da_config| {
        da_config.sender_address = setup.sequencer.da_address;
    });
    let rollup = rollup_builder
        .start()
        .await
        .expect("Impossible to start rollup");

    if wait_for_finalized_slot {
        // Give rollup to process a couple finalized blocks before starting accepting transactions.
        let mut finalized_slots_sub = rollup
            .api_client()
            .subscribe_finalized_slots()
            .await
            .expect("failed to subscribe to finalized slots");

        for _ in 0..FINALIZED_BLOCKS_AT_START {
            let _ = finalized_slots_sub.next().await;
        }
    }

    rollup
}

/// Helper for handling the dockerized hyperlane setup.
pub struct HyperlaneBuilder {
    image: GenericImage,
    rollup_port: Option<u16>,
    with_evm: bool,
    relayer: Option<PrivateKey>,
    validators: Vec<PrivateKey>,
}

impl HyperlaneBuilder {
    /// Sets up and pulls hyperlane image
    pub async fn setup_image() -> Self {
        let docker_image = env::var("CUSTOM_HLP_DOCKER_IMAGE");
        let has_custom_image = !matches!(docker_image, Err(env::VarError::NotPresent));

        // Current image is based on https://github.com/citizen-stig/hyperlane-monorepo/tree/nikolai/for-test
        // TODO: Migrate it to https://github.com/Sovereign-Labs/hyperlane-monorepo/ and later to upstream.
        let docker_image = docker_image
            .unwrap_or_else(|_| "ghcr.io/citizen-stig/hyperlane-agent:integration-2".into());
        let (name, tag) = docker_image
            .split_once(':')
            .unwrap_or((&docker_image, "latest"));

        let image = GenericImage::new(name, tag);

        // try to pull the image from registry before starting tests
        // but don't pull custom images, as they can be local and it would fail
        if !has_custom_image {
            let _ = image
                .clone()
                .pull_image()
                .await
                .expect("failed to pull image");
        }

        Self {
            image,
            rollup_port: None,
            with_evm: false,
            relayer: None,
            validators: vec![],
        }
    }

    /// Set rollup port hyperlane can reach out to.
    pub fn with_rollup_port(mut self, rollup_port: u16) -> Self {
        self.rollup_port = Some(rollup_port);
        self
    }

    /// Run relayer with specified key.
    pub fn with_relayer(mut self, relayer: &TestUser<TestSpec>) -> Self {
        self.relayer = Some(relayer.private_key.clone());
        self
    }

    /// Run validators with specified keys.
    pub fn with_validators<'a>(
        mut self,
        validators: impl IntoIterator<Item = &'a TestUser<TestSpec>>,
    ) -> Self {
        self.validators = validators
            .into_iter()
            .map(|user| &user.private_key)
            .cloned()
            .collect();
        self
    }

    /// Run evm counterparty that will send test messages to specified recipient
    pub fn with_evm_counterparty(mut self) -> Self {
        self.with_evm = true;
        self
    }

    /// Start the configured hyperlane network setup.
    pub async fn start(self) -> Hyperlane {
        let rollup_port = self
            .rollup_port
            .expect("Rollup port must be set before starting hyperlane");

        // evm counterparty must be started before agents
        // because they will try to reach out to it immediately.
        // the same goes for rollup, but we assume it's running knowing its port.
        let (evm_counter_party, anvil_port) = if self.with_evm {
            let evm_counter_party = EvmCounterParty::new(rollup_port).await;
            let anvil_port = evm_counter_party.anvil.port();
            (Some(evm_counter_party), anvil_port)
        } else {
            // Does not matter, default port going to do
            (None, ANVIL_PORT)
        };

        // Start container with just basic env and no processes
        let mut builder = self
            .image
            // map needed ports to localhost
            .with_exposed_port(ANVIL_PORT.tcp())
            .with_exposed_port(RELAYER_METRICS_PORT.tcp())
            .with_exposed_port(VALIDATOR_METRICS_PORT.tcp())
            // a bridge to the host system, to reach rollup from within container
            .with_host("host.docker.internal", Host::HostGateway)
            // test runtime uses fixed value for chain hash, this lets relayer know
            .with_env_var("SOV_TEST_UTILS_FIXED_CHAIN_HASH", "true")
            // default signing key for hyperlane cli and relayer in evm
            .with_env_var("HYP_KEY", ANVIL_ACCOUNTS[0].1)
            // setup agent config. NOTE: maybe use this in hyperlane-cli
            .with_copy_to(
                "/sov-agent-config.json",
                agent_config(rollup_port, anvil_port),
            )
            .with_env_var("CONFIG_FILES", "/sov-agent-config.json")
            // a dummy command because we will populate services by execs appropriately
            .with_cmd(["tail", "-f", "/dev/null"]);

        // The hyperlane CLI accesses GitHub APIs quite heavily for its GitHub hosted
        // registry, this can cause rate limiting in CI jobs. Include the github token
        // so we use authenticated requests to try avoid this
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            // `hyperlane` cli tool will use this env var by default as an auth token
            // if it is set.
            builder = builder.with_env_var("GH_AUTH_TOKEN", token);
        }

        let container = builder
            .start()
            .await
            .expect("Failed starting hyperlane image");

        // start all the hyperlane agents concurrently
        let has_relayer = self.relayer.is_some();
        let maybe_relayer_fut = if has_relayer {
            let fut = start_relayer(
                &container,
                self.relayer.unwrap(),
                evm_counter_party.is_some(),
            );
            Some(fut.boxed_local())
        } else {
            None
        };
        let validators_futs = self
            .validators
            .into_iter()
            .enumerate()
            .map(|(id, key)| start_validator(&container, id, key).boxed_local());

        let mut agents = join_all(
            maybe_relayer_fut
                .into_iter()
                .chain(validators_futs.into_iter()),
        )
        .await;

        let relayer = if has_relayer {
            Some(agents.remove(0))
        } else {
            None
        };

        Hyperlane {
            container,
            evm_counter_party,
            relayer,
            validators: agents,
        }
    }
}

pub(crate) struct EvmCounterParty {
    anvil: AnvilRunner,
    hyperlane_cli: HyperlaneCliRunner,
    pub evm_recipient: HexHash,
}

impl EvmCounterParty {
    async fn new(rollup_port: u16) -> Self {
        let anvil = AnvilRunner::new().await;
        let anvil_port = anvil.port();
        let hyperlane_cli = HyperlaneCliRunner::new(rollup_port, anvil_port);
        let hyperlane_deploy_start = std::time::Instant::now();
        let evm_recipient = hyperlane_cli.deploy_core().await;
        tracing::info!(time = ?hyperlane_deploy_start.elapsed(), "Hyperlane deployed");
        Self {
            anvil,
            hyperlane_cli,
            evm_recipient,
        }
    }
}

pub struct Hyperlane {
    // Keep ownership of the container, so it does not stopped before neeeded.
    #[allow(dead_code)]
    pub container: Container,
    pub evm_counter_party: Option<EvmCounterParty>,
    pub relayer: Option<ExecResult>,
    pub validators: Vec<ExecResult>,
}

impl Hyperlane {
    fn get_anvil(&self) -> &AnvilRunner {
        &self
            .evm_counter_party
            .as_ref()
            .expect("Cannot use without evm")
            .anvil
    }

    fn get_anvil_mut(&mut self) -> &mut AnvilRunner {
        &mut self
            .evm_counter_party
            .as_mut()
            .expect("Cannot use without evm")
            .anvil
    }

    fn get_hyperlane_cli(&self) -> &HyperlaneCliRunner {
        &self
            .evm_counter_party
            .as_ref()
            .expect("Cannot use without evm")
            .hyperlane_cli
    }

    /// Send test message from evm counterparty to sov test recipient
    pub async fn dispatch_msg_from_counterparty(&self, recipient: HexHash) -> EvmDispatchWithId {
        if self.evm_counter_party.is_none() {
            panic!("called dispatch_msg_from_counterparty without set up counterparty");
        }
        let dest_domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

        let anvil = self.get_anvil();

        // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/main/solidity/contracts/Mailbox.sol#L110
        let logs = anvil
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

    /// Searches the latest block on evm counterparty (where there's block per tx)
    /// and tries to extract the Mailbox Process event from it.
    pub async fn latest_message_on_counterparty(&mut self) -> EvmProcessWithId {
        let anvil = self.get_anvil_mut();
        // fetch logs in the latest block
        let logs: Vec<_> = anvil.rpc("eth_getLogs", json!([{}])).await;
        println!("LOGS: {logs:?}");
        EvmProcessWithId::new(logs)
    }

    /// Mines next block on the counterparty evm chain.
    ///
    /// Needed to finalize previous blocks for relayer to pick up txs.
    pub async fn mine_next_block_on_counterparty(&mut self) {
        if self.evm_counter_party.is_none() {
            panic!("Called mine next block on counterparty before its setup");
        }
        let anvil = &mut self
            .evm_counter_party
            .as_mut()
            .expect("Cannot use without evm")
            .anvil;

        anvil.rpc::<Value>("anvil_mine", json!([1])).await;
    }

    /// Create warp route for nativeETH on counterparty, enroll remote router to rollup,
    /// and return route address on counterparty.
    pub async fn deploy_warp_route_on_counterparty(&mut self, sovtest_route: HexHash) -> HexHash {
        if self.evm_counter_party.is_none() {
            panic!("Called warp init on counterparty before its setup");
        }

        let remote_router_id = self.get_hyperlane_cli().deploy_warp().await;

        let anvil = self.get_anvil_mut();

        let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
        anvil
            .cast_call(
                hex_hash_into_eth_addr(&remote_router_id),
                "enrollRemoteRouter(uint32,bytes32)",
                [
                    domain.to_string().as_str(),
                    sovtest_route.to_string().as_str(),
                ],
                Amount(0),
            )
            .await;

        remote_router_id
    }

    pub async fn send_warp_token_transfer_from_counterparty(
        &mut self,
        counterparty_route_id: HexHash,
        recipient: HexHash,
        amount: Amount,
    ) -> EvmDispatchWithId {
        if self.evm_counter_party.is_none() {
            panic!("called dispatch_msg_from_counterparty without set up counterparty");
        }
        let route_addr = HexString::new(counterparty_route_id.0[12..].try_into().unwrap());
        let destination = config_value!("HYPERLANE_BRIDGE_DOMAIN").to_string();

        let anvil = self.get_anvil();
        // https://github.com/hyperlane-xyz/hyperlane-monorepo/tree/c177c4733de52f8a2477ad74b46b3f1eebb5740b/solidity/contracts/token/libs/TokenRouter.sol#L54
        let logs = anvil
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

    pub async fn counterparty_balance_of(&mut self, address: HexHash) -> Amount {
        let addr = HexString(&address.0[12..]);
        let anvil = self.get_anvil_mut();
        let mut balance: String = anvil
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

    /// Searches latest block on evm counterparty (where there's block per tx)
    /// and tries to extract the event of native token received: (origin_domain, recipient)
    pub async fn latest_warp_transfer_on_counterparty(
        &mut self,
        token_addr: HexHash,
    ) -> (u32, HexHash) {
        let token_eth_addr = HexString(&token_addr.0[12..]);
        let anvil = self.get_anvil_mut();

        // fetch logs in latest block
        let logs: Vec<EvmLog> = anvil.rpc("eth_getLogs", json!([{}])).await;
        let log = logs
            .into_iter()
            .find(|log| log.address.0 == token_eth_addr.0)
            .unwrap();

        // first topic is event signature
        assert_eq!(log.topics.len(), 3);

        let origin_domain = domain_from_hexhash(log.topics[1]);
        (origin_domain, log.topics[2])
    }

    /// Prints container's stdout
    pub async fn print_stdout(&mut self) {
        // we don't have an option for no-follow stdout access
        // on `ExecResult`s, so this would hang infinitly waiting
        // for `exec`s to exit. Instead we give them at most 1s of
        // printing time each.
        println!("VALIDATORS: {}", self.validators.len());
        let has_relayer = self.relayer.is_some();
        for (n, val) in self
            .relayer
            .iter_mut()
            .chain(self.validators.iter_mut())
            .enumerate()
        {
            if n == 0 && has_relayer {
                println!("RELAYER\n");
            } else {
                println!("\n\nVALIDATOR {n}\n");
            }
            let exit_code = val.exit_code().await.unwrap();
            println!("EXIT CODE: {exit_code:?}");
            let _ = timeout(Duration::from_secs(3), async {
                println!("PRINTING STDOUT:");
                let mut stdout = val.stdout().lines();
                while let Some(line) = stdout.next_line().await.unwrap() {
                    println!("STDOUT: {line}");
                }
            })
            .await;
            let _ = timeout(Duration::from_secs(3), async {
                println!("PRINTING STDERR:");
                let mut stderr = val.stderr().lines();
                while let Some(line) = stderr.next_line().await.unwrap() {
                    println!("STDERR {line}");
                }
            })
            .await;
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EvmLog {
    address: EthAddress,
    /// First topic is keccak hash of event's signature
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
    fn new(logs: impl IntoIterator<Item = EvmLog>) -> Self {
        let mut logs = logs.into_iter().filter(|log| log.address == EVM_MAILBOX);
        let process = logs.next().unwrap();
        let process_id = logs.next().unwrap();

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

        // first 32 bytes is field's offset, always 0x20 for first field
        // next 32 bytes is the length of the field bytes
        let encoded_len = &dispatch.data.0[32..64];
        assert!(encoded_len.iter().take(28).all(|&byte| byte == 0));
        let message_len = u32::from_be_bytes(encoded_len[28..].try_into().unwrap());
        // next comes the field's data, with the length we just parsed, padded with 0' to the
        // mulitplier of 32
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

/// Starts a relayer in docker container
async fn start_relayer(
    container: &Container,
    private_key: PrivateKey,
    relay_evm: bool,
) -> ExecResult {
    let relay_chains = if relay_evm {
        "sovtest,ethtest"
    } else {
        "sovtest"
    };

    let sov_key = HexHash::new(private_key.as_bytes());
    let cmd = ExecCommand::new([
        "/app/relayer",
        "--db",
        "/app/relayer-db",
        // signer for the rollup
        "--chains.sovtest.signer.type",
        "sovereignKey",
        "--chains.sovtest.signer.key",
        &sov_key.to_string(),
        "--chains.sovtest.signer.accountType",
        "sovereign",
        "--chains.sovtest.signer.hrp",
        "sov",
        // signer for the counterparty
        "--chains.ethtest.signer.key",
        ANVIL_ACCOUNTS[0].1,
        // chains to relay
        "--relayChains",
        relay_chains,
        // allow using validator signatures from local fs
        "--allowLocalCheckpointSyncers",
        "true",
        "--metrics-port",
        RELAYER_METRICS_PORT.to_string().as_str(),
        "--log.level",
        "debug",
        "--log.format",
        "pretty",
    ])
    // Options:
    // 1. INFO "Agent relayer starting up" - before settings, so any error in settings is going to be missed
    // 2. INFO "Creating db" - settings have been parsed, but won't catch failure of db or sysargs
    // 3. DEBUG "Relayer startup duration measurement" - "fully initialized": printed after initialization is completed, but require debug.
    .with_cmd_ready_condition(CmdWaitFor::message_on_stdout("fully initialized"));

    container.exec(cmd).await.expect("starting relayer failed")
}

/// Starts a relayer in docker container
async fn start_validator(
    container: &Container,
    val_id: usize,
    private_key: PrivateKey,
) -> ExecResult {
    // set the known port only for first validator, and let os choose random one for rest
    let metrics_port = if val_id == 0 {
        VALIDATOR_METRICS_PORT
    } else {
        0
    };

    let val_db_path = format!("/app/validator-{val_id}-db");
    let val_sigs_path = format!("/app/validator-{val_id}/signatures");
    let val_eth_key = ANVIL_ACCOUNTS[val_id + 1].1;

    // make directories for db and signatures
    let mkdir_cmd = ExecCommand::new(["mkdir", "-p", val_db_path.as_str(), val_sigs_path.as_str()]);
    // TODO: check status!
    let _mkdir_result = container.exec(mkdir_cmd).await.unwrap();

    let sov_key = HexHash::new(private_key.as_bytes());
    let cmd = ExecCommand::new([
        "/app/validator",
        // save signatures on local fs
        "--checkpointSyncer.type",
        "localStorage",
        // path to save signatures to
        "--checkpointSyncer.path",
        val_sigs_path.as_str(),
        // a database for validator storage
        "--db",
        val_db_path.as_str(),
        // a chain of which messages are going to be signed
        "--originChainName",
        "sovtest",
        // key for the checkpoints signatures
        "--validator.key",
        val_eth_key,
        // signer for the rollup
        "--chains.sovtest.signer.type",
        "sovereignKey",
        "--chains.sovtest.signer.key",
        &sov_key.to_string(),
        "--chains.sovtest.signer.accountType",
        "sovereign",
        "--chains.sovtest.signer.hrp",
        "sov",
        "--metrics-port",
        metrics_port.to_string().as_str(),
        "--log.level",
        "debug",
        "--log.format",
        "pretty",
    ])
    .with_cmd_ready_condition(CmdWaitFor::message_on_stdout("starting server on"));

    // run validator
    container
        .exec(cmd)
        .await
        .expect("starting validator failed")
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub async fn get_docker_gateway_ip() -> String {
    let bridge_info = docker_client_instance()
        .await
        .unwrap()
        .inspect_network(
            "bridge",
            None::<testcontainers::bollard::query_parameters::InspectNetworkOptions>,
        )
        .await
        .unwrap();
    bridge_info
        .ipam
        .expect("no IPAM driver found")
        .config
        .expect("IPAM has no configuration")
        .into_iter()
        .find_map(|conf| conf.gateway)
        .expect("No gateway config in IPAM")
}

// parses eth addr 0x(40 chars hex) into HexHash
pub fn parse_eth_addr(addr: &str) -> HexHash {
    // TODO: use sov-address with proper feature?
    let address: EthAddress = addr.trim().parse().unwrap();
    let mut res = [0; 32];
    res[12..].copy_from_slice(&address.0);
    res.into()
}

pub fn hex_hash_into_eth_addr(hex_hash: &HexHash) -> EthAddress {
    let mut res = [0; 20];
    res[..].copy_from_slice(&hex_hash.0[12..]);
    res.into()
}

fn domain_from_hexhash(hash: HexHash) -> u32 {
    assert!(hash.0[0..28].iter().all(|&b| b == 0));
    u32::from_be_bytes(hash.0[28..].try_into().unwrap())
}
