mod docker;
mod evm;
mod hyperlane_cli;

use std::env;

use super::configs::agent_config;
use super::preferred_sequencer_runtime::{GenesisConfig, TestRuntime};
use crate::with_agent::helpers::docker::print_logs_from_exec_result;
use crate::with_agent::helpers::evm::{
    EvmCounterParty, EvmDispatchWithId, EvmProcessWithId, ANVIL_PORT,
};
use futures::future::join_all;
use futures::{FutureExt, StreamExt};
use sov_bank::Amount;
use sov_hyperlane_integration::EthAddress;
use sov_modules_api::{CryptoSpec, HexHash, HexString, Spec};
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{RtAgnosticBlueprint, TestProver, TestSequencer, TestSpec, TestUser};
use testcontainers::core::{CmdWaitFor, ExecCommand, ExecResult};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

pub type RollupBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;
pub type TestRollupBuilder = RollupBuilder<RollupBlueprint>;
pub type PrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

type Container = ContainerAsync<GenericImage>;

pub const FINALIZED_BLOCKS_AT_START: usize = 3;
pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 10;
/// Use `container.get_host_port_ipv4(RELAYER_METRICS_PORT)` to get metrics
pub const RELAYER_METRICS_PORT: u16 = 9091;
pub const VALIDATOR_METRICS_PORT: u16 = 9097;
/// Domain id of the evm counterparty chain
/// Shouldn't match any known hyperlane network, otherwise it fails.
pub const EVM_DOMAIN: u32 = 31337_90210;
pub const EVM_CHAIN_ID: u32 = 31337;
/// Address of the mailbox on evm counterparty chain.
/// Derived from the deployer in hyperlane-cli
/// 0x8a791620dd6260079bf849dc5567adc3f2fdc318
pub const EVM_MAILBOX: EthAddress = HexString([
    18, 151, 81, 115, 184, 127, 117, 149, 238, 69, 223, 251, 42, 184, 18, 236, 229, 150, 191, 132,
]);
/// Fixed Eth keys created by anvil. They don't change. Each address is funded 1000ETH
// run `docker run --rm ghcr.io/foundry-rs/foundry:v1.1.0 anvil` to see all keys
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

// Explicitly specify relayer account for readability
pub const RELAYER_ACCOUNT: (&str, &str) = ANVIL_ACCOUNTS[0];
// Use a separate account for hyperlane CLI deployments to avoid nonce conflicts with relayer
pub const DEPLOYER_ACCOUNT: (&str, &str) = ANVIL_ACCOUNTS[9];

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
    setup: Setup,
    wait_for_finalized_slot: bool,
) -> TestRollup<RollupBlueprint> {
    let axum_bind_ip = if cfg!(target_os = "macos") {
        // MacOS runs docker inside the VM, so returned gateway IP does not match any address on the host.
        // Test containers already expose all the ports to `0.0.0.0` so this does not increase security risk significantly.
        // If better solution exists, happy to apply it
        "0.0.0.0".to_string()
    } else {
        docker::get_docker_gateway_ip().await
    };
    let rollup_builder = TestRollupBuilder::new(
        GenesisSource::CustomParams(setup.genesis_config.clone().into_genesis_params()),
        sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        DEFAULT_FINALIZATION_BLOCKS,
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

        // Current image is based on https://github.com/Sovereign-Labs/hyperlane-monorepo/tree/integration-2025-08-27-rebase branch
        let docker_image = docker_image
            .unwrap_or_else(|_| "ghcr.io/sovereign-labs/hyperlane-agent:integration-2".into());
        let (name, tag) = docker_image
            .split_once(':')
            .unwrap_or((&docker_image, "latest"));
        tracing::info!(%name, %tag, "Using hyperlane agent docker image");

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

    /// Run relayer with a specified key.
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

        let host_address = if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
            "host.docker.internal".to_string()
        } else {
            // On Linux, get the Docker bridge network gateway IP
            docker::get_docker_gateway_ip().await
        };

        // evm counterparty must be started before agents
        // because they will try to reach out to it immediately.
        // the same goes for rollup, but we assume it's running knowing its port.
        let (evm_counter_party, anvil_port) = if self.with_evm {
            let evm_counter_party = EvmCounterParty::new(rollup_port, &host_address).await;
            let anvil_port = evm_counter_party.anvil.port();
            (Some(evm_counter_party), anvil_port)
        } else {
            // Does not matter, default port going to do
            (None, ANVIL_PORT)
        };

        // Start container with just basic env and no processes
        let builder = self
            .image
            // test runtime uses fixed value for chain hash, this lets relayer know
            .with_env_var("SOV_TEST_UTILS_FIXED_CHAIN_HASH", "true")
            // default signing key for hyperlane cli and relayer in evm
            .with_env_var("HYP_KEY", RELAYER_ACCOUNT.1)
            // setup agent config. NOTE: maybe use this in hyperlane-cli
            .with_copy_to(
                "/agent-config.json",
                agent_config(rollup_port, anvil_port, &host_address),
            )
            .with_env_var("CONFIG_FILES", "/agent-config.json")
            // a dummy command because we will populate services by execs appropriately
            .with_cmd(["tail", "-f", "/dev/null"]);

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

pub struct Hyperlane {
    // Keep ownership of the container, so it does not stopped before neeeded.
    #[allow(dead_code)]
    pub container: Container,
    pub evm_counter_party: Option<EvmCounterParty>,
    pub relayer: Option<ExecResult>,
    pub validators: Vec<ExecResult>,
}

impl Hyperlane {
    /// Send test message from evm counterparty to sov test recipient
    pub async fn dispatch_msg_from_counterparty(&self, recipient: HexHash) -> EvmDispatchWithId {
        self.evm_counter_party
            .as_ref()
            .expect("called dispatch_msg_from_counterparty without set up counterparty")
            .dispatch_msg_to(recipient)
            .await
    }

    /// Searches the latest block on evm counterparty (where there's block per tx)
    /// and tries to extract the Mailbox Process event from it.
    pub async fn latest_message_on_counterparty(&mut self) -> EvmProcessWithId {
        self.evm_counter_party
            .as_mut()
            .expect("Called latest message on counterparty before its setup")
            .latest_message()
            .await
    }

    /// Mines next block on the counterparty evm chain.
    ///
    /// Needed to finalize previous blocks for relayer to pick up txs.
    pub async fn mine_next_block_on_counterparty(&mut self) {
        self.evm_counter_party
            .as_mut()
            .expect("Called mine next block on counterparty before its setup")
            .mine_block()
            .await;
    }

    /// Create a warp route for nativeETH on counterparty, enroll remote router to rollup,
    /// and return route address on counterparty.
    pub async fn deploy_warp_route_on_counterparty(&mut self, sovtest_route: HexHash) -> HexHash {
        self.evm_counter_party
            .as_mut()
            .expect("Called warp init on counterparty before its setup")
            .deploy_warp_route(sovtest_route)
            .await
    }

    pub async fn send_warp_token_transfer_from_counterparty(
        &mut self,
        counterparty_route_id: HexHash,
        recipient: HexHash,
        amount: Amount,
    ) -> EvmDispatchWithId {
        self.evm_counter_party
            .as_mut()
            .expect("called dispatch_msg_from_counterparty without set up counterparty")
            .send_warp_token_transfer(counterparty_route_id, recipient, amount)
            .await
    }

    pub async fn counterparty_balance_of(&mut self, address: HexHash) -> Amount {
        self.evm_counter_party
            .as_mut()
            .expect("called counterparty_balance_of before setting its setup")
            .balance_of(address)
            .await
    }

    /// Searches the latest block on evm counterparty (where there's block per tx)
    /// and tries to extract the event of native token received: (origin_domain, recipient)
    pub async fn latest_warp_transfer_on_counterparty(
        &mut self,
        token_addr: HexHash,
    ) -> (u32, HexHash) {
        self.evm_counter_party
            .as_mut()
            .expect("called latest_warp_transfer_on_counterparty before its setup")
            .latest_warp_transfer(token_addr)
            .await
    }

    /// Prints container's stdout
    pub async fn print_stdout(&mut self) {
        if let Some(evm_counter_party) = self.evm_counter_party.as_ref() {
            evm_counter_party.print_logs().await;
        }
        // we don't have an option for no-follow stdout access
        // on `ExecResult`s, so this would hang infinitely waiting
        // for `exec`s to exit. Instead, we give them at most 1s of
        // printing time each.
        let has_relayer = self.relayer.is_some();
        for (n, val) in self
            .relayer
            .iter_mut()
            .chain(self.validators.iter_mut())
            .enumerate()
        {
            let name = if n == 0 && has_relayer {
                "relayer".to_string()
            } else {
                format!("validator-{n}")
            };
            print_logs_from_exec_result(&name, val, std::time::Duration::from_secs(1)).await;
        }
    }
}

/// Starts a relayer in a docker container
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
        RELAYER_ACCOUNT.1,
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

/// Starts a relayer in a docker container
async fn start_validator(
    container: &Container,
    val_id: usize,
    private_key: PrivateKey,
) -> ExecResult {
    // set the known port only for the first validator, and let os choose random one for rest
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
    let mkdir_result = container.exec(mkdir_cmd).await.unwrap();
    let mut exit_code = mkdir_result
        .exit_code()
        .await
        .expect("Failed to get exit code for validator directory creation");
    for _ in 1..50 {
        if exit_code.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        exit_code = mkdir_result
            .exit_code()
            .await
            .expect("Failed to get exit code for validator directory creation");
    }

    if exit_code != Some(0) {
        panic!("Failed to create directory for validator {val_id}, exit code: {exit_code:?}",);
    }

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

    container
        .exec(cmd)
        .await
        .expect("starting validator failed")
}

// parses eth addr 0x(40 chars hex) into HexHash
pub fn parse_eth_addr(addr: &str) -> HexHash {
    // TODO: use sov-address with proper feature?
    let address: EthAddress = addr.trim().parse().unwrap();
    let mut res = [0; 32];
    res[12..].copy_from_slice(&address.0);
    res.into()
}
