use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::join_all;
use futures::{FutureExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sov_bank::Amount;
use sov_hyperlane_integration::{EthAddress, Message};
use sov_mock_da::BlockProducingConfig;
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

use super::configs::{
    agent_config, core_config, ethtest_metadata, sovtest_addresses, sovtest_metadata,
    warp_route_deployment,
};
use super::preferred_sequencer_runtime::{GenesisConfig, TestRuntime};
use crate::with_agent::configs::warp_route_config;

pub type RollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;
pub type TestRollupBuilder = RollupBuilder<RollupBlueprint, PathBuf>;
pub type PrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

type Container = ContainerAsync<GenericImage>;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 400;
pub const FINALIZED_BLOCKS_AT_START: usize = 3;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};
pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 10;
/// Use `container.get_host_port_ipv4(RELAYER_METRICS_PORT)` to get metrics
pub const ANVIL_PORT: u16 = 8545;
pub const RELAYER_METRICS_PORT: u16 = 9091;
pub const VALIDATOR_METRICS_PORT: u16 = 9097;
/// Domain id of the evm counterparty chain
pub const EVM_DOMAIN: u32 = 31337;
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
    )
    .set_config(|config| {
        config.automatic_batch_production = true;
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
    rollup_recipient: Option<HexHash>,
    relayer: Option<PrivateKey>,
    validators: Vec<PrivateKey>,
}

impl HyperlaneBuilder {
    /// Sets up and pulls hyperlane image
    pub async fn setup_image() -> Self {
        let docker_image = env::var("CUSTOM_HLP_DOCKER_IMAGE");
        let has_custom_image = !matches!(docker_image, Err(env::VarError::NotPresent));

        let docker_image =
            docker_image.unwrap_or_else(|_| "ghcr.io/eigerco/hyperlane:latest".into());
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
            rollup_recipient: None,
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
    pub fn with_evm_counterparty(mut self, rollup_recipient: HexHash) -> Self {
        self.rollup_recipient = Some(rollup_recipient);
        self
    }

    /// Start the configured hyperlane network setup.
    pub async fn start(self) -> Hyperlane {
        let rollup_port = self.rollup_port.expect("Rollup port must be set");

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
            // setup agent config
            .with_copy_to("/agent-config.json", agent_config(rollup_port))
            .with_env_var("CONFIG_FILES", "/agent-config.json")
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

        // evm counterparty must be started before agents
        // because they will try to reach out to it immediately.
        // same goes for rollup, but we assume its runnig knowing its port.
        let (anvil, evm_recipient) = if let Some(rollup_recipient) = self.rollup_recipient {
            let (anvil, evm_recipient) =
                start_evm_counterparty(&container, rollup_recipient, rollup_port).await;
            (Some(anvil), Some(evm_recipient))
        } else {
            (None, None)
        };

        // start all the hyperlane agents concurrently
        let has_relayer = self.relayer.is_some();
        let maybe_relayer_fut = if has_relayer {
            let fut = start_relayer(&container, self.relayer.unwrap(), anvil.is_some());
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
            anvil,
            evm_recipient,
            relayer,
            validators: agents,
        }
    }
}

pub struct Hyperlane {
    pub container: Container,
    pub anvil: Option<ExecResult>,
    pub evm_recipient: Option<HexHash>,
    pub relayer: Option<ExecResult>,
    pub validators: Vec<ExecResult>,
}

impl Hyperlane {
    /// Send test message from evm counterparty to sov test recipient
    pub async fn dispatch_msg_from_counterparty(&self) -> (Message, HexHash) {
        if self.anvil.is_none() {
            panic!("called dispatch_msg_from_counterparty without set up counterparty");
        }

        let output = self
            .container
            .exec(ExecCommand::new([
                "hyperlane",
                "send",
                "message",
                "--origin",
                "ethtest",
                "--destination",
                "sovtest",
                // don't wait for confirmation of delivery
                "--quick",
            ]))
            .await
            .unwrap()
            .stdout_to_vec()
            .await
            .unwrap();

        let output = String::from_utf8_lossy(&output);
        // skip everything up to Message info
        let lines: Vec<_> = output
            .lines()
            .skip_while(|&line| line != "Message:")
            .collect();

        extract_message_from_logs(&lines, true)
    }

    /// Searches latest block on evm counterparty (where there's block per tx)
    /// and tries to extract the Mailbox Process event from it.
    pub async fn latest_message_on_counterparty(&self) -> EvmProcessWithId {
        // fetch logs in latest block
        let logs: Vec<EvmLog> = anvil_rpc(&self.container, "eth_getLogs", json!([{}])).await;
        let mut logs = logs.into_iter().filter(|log| log.address == EVM_MAILBOX);
        let process = logs.next().unwrap();
        let process_id = logs.next().unwrap();

        // we should only have 2 logs
        assert!(logs.next().is_none());

        EvmProcessWithId::new(&process.topics, &process_id.topics)
    }

    /// Mines next block on the counterparty evm chain.
    ///
    /// Needed to finalize previous blocks for relayer to pick up txs.
    pub async fn mine_next_block_on_counterparty(&self) {
        if self.anvil.is_none() {
            panic!("Called mine next block on counterparty before its setup");
        }

        anvil_rpc::<Value>(&self.container, "anvil_mine", json!([1])).await;
    }

    /// Create warp route for nativeETH on counterparty, enroll remote router to rollup,
    /// and return route address on counterparty.
    pub async fn deploy_warp_route_on_counterparty(
        &self,
        sovtest_route: HexHash,
        sovtest_decimals: u8,
    ) -> HexHash {
        if self.anvil.is_none() {
            panic!("Called warp init on counterparty before its setup");
        }

        // deploy warp route on evm counterparty
        let warp_config = warp_route_config(sovtest_route);
        exec_in_bash(
            &self.container,
            format!("echo '{warp_config}' > configs/warp-route-deployment.yaml"),
        )
        .await;
        let mut res = self
            .container
            .exec(ExecCommand::new(["hyperlane", "warp", "deploy", "--yes"]))
            .await
            .unwrap();

        let stdout = res.stdout_to_vec().await.unwrap();
        let stdout = String::from_utf8_lossy(&stdout);
        if res.exit_code().await.unwrap().unwrap() != 0 {
            println!("hyperlane warp deploy stdout: {stdout}");
            panic!("hyperlane deployment on evm chain failed");
        }

        // parse ethtest route address from logs
        let ethtest_route = stdout
            .lines()
            .find(|line| line.contains("addressOrDenom"))
            .unwrap()
            .split("\"")
            .nth(1)
            .unwrap();
        let ethtest_route = parse_eth_addr(ethtest_route);

        // now call the hyperlane warp apply for the same config,
        // as hyperlane warp deploy skips `remoteRouters`
        let mut res = self
            .container
            .exec(ExecCommand::new([
                "hyperlane",
                "warp",
                "apply",
                "--symbol",
                "nativeETH",
                "--config",
                "configs/warp-route-deployment.yaml",
                "--yes",
            ]))
            .await
            .unwrap();

        let stdout = res.stdout_to_vec().await.unwrap();
        let stdout = String::from_utf8_lossy(&stdout);
        if res.exit_code().await.unwrap().unwrap() != 0 {
            println!("hyperlane warp apply stdout: {stdout}");
            let stderr = res.stderr_to_vec().await.unwrap();
            let stderr = String::from_utf8_lossy(&stderr);
            println!("hyperlane warp apply stderr: {stderr}");
            panic!("hyperlane deployment on evm chain failed");
        }

        // put the config for `hyperlane warp send` command that includes the rollup
        // as `warp deploy` only had ethtest configuration.
        let warp_deployment = warp_route_deployment(ethtest_route, sovtest_route, sovtest_decimals);
        exec_in_bash(
            &self.container,
            format!("echo '{warp_deployment}' > ~/.hyperlane/deployments/warp_routes/nativeETH/ethtest-config.yaml"),
        ).await;

        ethtest_route
    }

    pub async fn send_warp_token_transfer_from_counterparty(
        &self,
        recipient: HexHash,
        amount: Amount,
    ) -> (Message, HexHash) {
        if self.anvil.is_none() {
            panic!("called dispatch_msg_from_counterparty without set up counterparty");
        }

        // `warp send` currently only supports routes between evm chains.
        // `hyperlane-cli` thinks rollup uses "ethereum" protocol (see [`sovtest_metadata`]),
        // however we still need to run slightly patched version that does 2 things differently:
        // - runs pre-transfer checks only on origin chain
        // - accepts hyperlane addresses in addition to ethereum addresses
        // <https://github.com/eigerco/hyperlane-monorepo/commit/ca16901f7a4a86d041fb5f052bccc33deee8f4c4>
        let output = self
            .container
            .exec(ExecCommand::new([
                "hyperlane",
                "warp",
                "send",
                "--symbol",
                "nativeETH",
                "--origin",
                "ethtest",
                "--destination",
                "sovtest",
                "--amount",
                &amount.to_string(),
                "--recipient",
                &recipient.to_string(),
                // don't wait for confirmation of delivery
                "--quick",
            ]))
            .await
            .unwrap()
            .stdout_to_vec()
            .await
            .unwrap();

        let output = String::from_utf8_lossy(&output);
        // skip everything up to Message info
        let lines: Vec<_> = output
            .lines()
            .skip_while(|&line| line != "Message:")
            .collect();

        // don't extract the body
        extract_message_from_logs(&lines, false)
    }

    pub async fn counterparty_balance_of(&self, address: HexHash) -> Amount {
        let addr = HexString(&address.0[12..]);
        let mut balance: String = anvil_rpc(
            &self.container,
            "eth_getBalance",
            json!([addr.to_string(), "latest"]),
        )
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
        &self,
        token_addr: HexHash,
    ) -> (u32, HexHash) {
        let token_eth_addr = HexString(&token_addr.0[12..]);

        // fetch logs in latest block
        let logs: Vec<EvmLog> = anvil_rpc(&self.container, "eth_getLogs", json!([{}])).await;
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

            let _ = timeout(Duration::from_secs(1), async {
                let mut stdout = val.stdout().lines();
                while let Some(line) = stdout.next_line().await.unwrap() {
                    println!("{line}");
                }
            })
            .await;
        }
    }
}

#[derive(Debug, Deserialize)]
struct EvmLog {
    address: EthAddress,
    topics: Vec<HexHash>,
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
    /// Reconstruct combined process event from contract log's topics.
    fn new(dispatch_topics: &[HexHash], dispatch_id_topics: &[HexHash]) -> Self {
        // Topics are events, where first topic is event signature,
        // followed by event's fields in order.
        //
        // Fields on evm have the same order as our events
        // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/7656fe1c3865f817d68971ed3c8b939376065283/solidity/contracts/interfaces/IMailbox.sol#L29-L45
        assert_eq!(dispatch_topics.len(), 4);
        assert_eq!(dispatch_id_topics.len(), 2);

        let origin_domain = domain_from_hexhash(dispatch_topics[1]);

        EvmProcessWithId {
            origin_domain,
            sender_address: dispatch_topics[2],
            recipient_address: dispatch_topics[3],
            id: dispatch_id_topics[1],
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

    let cmd = ExecCommand::new([
        // relayer command
        "relayer",
        // database locations
        "--db",
        "/relayer-db",
        // signer for the rollup
        "--chains.sovtest.signer.key",
        format!("0x{}", private_key.as_hex()).as_str(),
        // signer for the counterparty
        "--chains.ethtest.signer.key",
        ANVIL_ACCOUNTS[0].1,
        // chains to relay
        "--relayChains",
        relay_chains,
        // allow using validator signatures from local fs
        "--allowLocalCheckpointSyncers",
        "true",
        // port for metrics
        "--metrics-port",
        RELAYER_METRICS_PORT.to_string().as_str(),
    ])
    .with_cmd_ready_condition(CmdWaitFor::message_on_stdout("Agent relayer starting up"));

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

    let val_db_path = format!("/validator{val_id}/db");
    let val_sigs_path = format!("/validator{val_id}/signatures");
    let val_eth_key = ANVIL_ACCOUNTS[val_id + 1].1;

    // make directories for db and signatures
    let mkdir_cmd = ExecCommand::new(["mkdir", "-p", val_db_path.as_str(), val_sigs_path.as_str()]);
    container.exec(mkdir_cmd).await.unwrap();

    let cmd = ExecCommand::new([
        // validator command
        "validator",
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
        "--chains.sovtest.signer.key",
        format!("0x{}", private_key.as_hex()).as_str(),
        // port for metrics
        "--metrics-port",
        metrics_port.to_string().as_str(),
    ])
    .with_cmd_ready_condition(CmdWaitFor::message_on_stdout("Agent validator starting up"));

    // run validator
    container
        .exec(cmd)
        .await
        .expect("starting validator failed")
}

/// Run Evm counterparty chain in docker.
///
/// Takes an address of sov test recipient, that can be used as a target of test
/// messages sent by `hyperlane-cli` and returns an address of evm test recipient
async fn start_evm_counterparty(
    container: &Container,
    rollup_recipient: HexHash,
    rollup_port: u16,
) -> (ExecResult, HexHash) {
    let anvil = container
        .exec(ExecCommand::new([
            "anvil",
            "--host",
            "0.0.0.0",
            "--port",
            &ANVIL_PORT.to_string(),
        ]))
        .await
        .unwrap();

    // Create chains configuration files for `hyperlane-cli`
    let chains_dir = "/root/.hyperlane/chains";
    let sovtest_config = sovtest_metadata(rollup_port);
    let ethtest_config = ethtest_metadata();
    for (chain, config) in [("sovtest", sovtest_config), ("ethtest", ethtest_config)] {
        exec_in_bash(
            container,
            format!("mkdir -p {chains_dir}/{chain}; echo '{config}' > {chains_dir}/{chain}/metadata.yaml")
        )
        .await;
    }

    // core config of hyperlane-cli, see `core_config`
    let core_config = core_config(ANVIL_ACCOUNTS[0].0.parse().unwrap());
    exec_in_bash(
        container,
        format!("mkdir configs && echo '{core_config}' > configs/core-config.yaml"),
    )
    .await;

    // Deploy smart contracts on ethereum and create
    // `~/.hyperlane/chains/ethtest/addresses.yaml
    let mut res = container
        .exec(ExecCommand::new([
            "hyperlane",
            "core",
            "deploy",
            "--chain",
            "ethtest",
            "--yes",
        ]))
        .await
        .unwrap();

    let stderr = res.stderr_to_vec().await.unwrap();
    let stdout = res.stdout_to_vec().await.unwrap();
    if res.exit_code().await.unwrap().unwrap() != 0 {
        println!("STDERR:\n{}", String::from_utf8_lossy(&stderr));
        println!("STDOUT:\n{}", String::from_utf8_lossy(&stdout));
        panic!("hyperlane deployment on evm chain failed");
    }

    // create `~/.hyperlane/chains/sovtest/addresses.yaml
    let sov_addresses = sovtest_addresses(rollup_recipient);
    exec_in_bash(
        container,
        format!("echo '{sov_addresses}' > {chains_dir}/sovtest/addresses.yaml"),
    )
    .await;

    // get ethereum test recipient
    let output = container
        .exec(ExecCommand::new([
            "awk",
            "-F",
            "\"",
            "/testRecipient/ { print $2 }",
            &format!("{chains_dir}/ethtest/addresses.yaml"),
        ]))
        .await
        .unwrap()
        .stdout_to_vec()
        .await
        .unwrap();
    let output = String::from_utf8_lossy(&output);

    (anvil, parse_eth_addr(&output))
}

/// runs docker exec <container> bash -c "cmd"
async fn exec_in_bash(container: &Container, cmd: impl AsRef<str>) -> ExecResult {
    let bash_c = ExecCommand::new(["bash", "-c", cmd.as_ref()]);
    container.exec(bash_c).await.unwrap()
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub async fn get_docker_gateway_ip() -> String {
    let bridge_info = docker_client_instance()
        .await
        .unwrap()
        .inspect_network::<String>("bridge", None)
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
    let address: HexString<[u8; 20]> = addr.trim().parse().unwrap();
    let mut res = [0; 32];
    res[12..].copy_from_slice(&address.0);

    res.into()
}

pub async fn anvil_rpc<T: DeserializeOwned>(
    container: &Container,
    method: &str,
    params: Value,
) -> T {
    static ID: AtomicUsize = AtomicUsize::new(0);
    let port = container.get_host_port_ipv4(ANVIL_PORT).await.unwrap();
    let resp = reqwest::Client::new()
        .post(format!("http://localhost:{port}"))
        .json(&json!({
            "id": ID.fetch_add(1, Ordering::Relaxed),
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
        panic!("Errors calling anvil jrpc: {error:?}");
    }

    serde_json::from_value(resp["result"].clone()).unwrap()
}

fn extract_message_from_logs(logs: &[&str], with_body: bool) -> (Message, HexHash) {
    let extract_field = |pattern, split| {
        logs.iter()
            .find(|l| l.contains(pattern))
            .and_then(|l| l.split(split).nth(1))
            .unwrap()
            .trim()
    };
    // we extract from two patterns and take second element
    // - '   something: 1234' using ': '
    // - '   something: "0x123...224"' using '"' to remove quotes
    let message_id = extract_field("id: ", "\"").parse().unwrap();
    let message = Message {
        version: extract_field("version: ", ": ").parse().unwrap(),
        nonce: extract_field("nonce: ", ": ").parse().unwrap(),
        origin_domain: extract_field("origin: ", ": ").parse().unwrap(),
        sender: extract_field("sender: ", "\"").parse().unwrap(),
        dest_domain: extract_field("destination: ", ": ").parse().unwrap(),
        recipient: extract_field("recipient: ", "\"").parse().unwrap(),
        // if body is too long, it's gonna be split into multiple lines
        // and harder to parse. This happens eg. in case of warp. Since it
        // is not needed in this case, it's simpler to just skip it conditionally
        body: if with_body {
            extract_field("body: ", "\"").parse().unwrap()
        } else {
            HexString::new(vec![])
        },
    };

    (message, message_id)
}

fn domain_from_hexhash(hash: HexHash) -> u32 {
    assert!(hash.0[0..28].iter().all(|&b| b == 0));
    u32::from_be_bytes(hash.0[28..].try_into().unwrap())
}
