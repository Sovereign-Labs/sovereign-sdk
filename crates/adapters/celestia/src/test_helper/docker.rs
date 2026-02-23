use std::borrow::Cow;
use std::str::FromStr;
use std::time::Duration;

use crate::config::{
    default_api_request_timeout_secs, default_background_stat_polling_interval_secs,
    default_factor, default_max_delay_ms, default_max_times, default_min_delay_ms,
    default_request_timeout_seconds, default_safe_lead_time_ms, default_tx_priority,
    default_tx_status_polling_millis,
};
use crate::verifier::address::CelestiaAddress;
use crate::{CelestiaConfig, CelestiaService};
use anyhow::{anyhow, Context};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use testcontainers::core::{ExecCommand, Host, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image, ImageExt};
use tokio::time::sleep;
use uuid::Uuid;

const VALIDATOR_IMAGE: &str = "ghcr.io/sovereign-labs/celestia-validator-devnet";
const VALIDATOR_TAG: &str = "v6.2.2-mocha";
const BRIDGE_IMAGE: &str = "ghcr.io/sovereign-labs/celestia-bridge-devnet";
const BRIDGE_TAG: &str = "v0.28.2-mocha";
const VALIDATOR_GRPC_PORT: u16 = 9090;
const BRIDGE_RPC_PORT: u16 = 26658;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

pub struct CelestiaValidator;

impl Image for CelestiaValidator {
    fn name(&self) -> &str {
        VALIDATOR_IMAGE
    }

    fn tag(&self) -> &str {
        VALIDATOR_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![
            WaitFor::healthcheck(),
            WaitFor::message_on_either_std("Genesis hash has been saved"),
        ]
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        vec![("NO_COLOR", "1")]
    }
}

pub struct CelestiaBridge;

impl Image for CelestiaBridge {
    fn name(&self) -> &str {
        BRIDGE_IMAGE
    }

    fn tag(&self) -> &str {
        BRIDGE_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![
            WaitFor::message_on_either_std("JWT token has been saved"),
            WaitFor::healthcheck(),
        ]
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        vec![("NO_COLOR", "1")]
    }
}

/// A collection of validator and node containers, that can be used for testing.
pub struct CelestiaDevNode {
    validator: ContainerAsync<CelestiaValidator>,
    bridge: ContainerAsync<CelestiaBridge>,
}

impl CelestiaDevNode {
    pub async fn start() -> anyhow::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let start = std::time::Instant::now();
        let suffix = Uuid::new_v4().to_string();

        let network = format!("celestia-test-{suffix}");
        let validator_name = format!("celestia-validator-{suffix}");
        let bridge_name = format!("celestia-bridge-{suffix}");
        let credentials_volume = format!("celestia-credentials-{suffix}");
        let genesis_volume = format!("celestia-genesis-{suffix}");

        tracing::debug!(
            %network,
            %validator_name,
            %bridge_name,
            %credentials_volume,
            %genesis_volume,
            "Preparing Celestia docker resources"
        );

        let validator = CelestiaValidator
            .with_network(&network)
            .with_container_name(&validator_name)
            .with_mount(Mount::volume_mount(
                credentials_volume.clone(),
                "/credentials",
            ))
            .with_mount(Mount::volume_mount(genesis_volume.clone(), "/genesis"))
            .with_startup_timeout(STARTUP_TIMEOUT)
            .start()
            .await
            .context("failed to start validator container")?;

        let validator_grpc_port = validator
            .get_host_port_ipv4(VALIDATOR_GRPC_PORT)
            .await
            .context("failed to obtain validator gRPC port")?;
        tracing::info!(
            id = validator.id(),
            port = validator_grpc_port,
            time = ?start.elapsed(),
            "Validator container started successfully"
        );

        let validator_ip = validator
            .get_bridge_ip_address()
            .await
            .context("failed to resolve validator bridge IP")?;

        let bridge = CelestiaBridge
            .with_network(&network)
            .with_container_name(&bridge_name)
            .with_host("validator", Host::Addr(validator_ip))
            .with_mount(Mount::volume_mount(
                credentials_volume.clone(),
                "/credentials",
            ))
            .with_mount(Mount::volume_mount(genesis_volume.clone(), "/genesis"))
            .start()
            .await
            .context("failed to start bridge container")?;

        let rpc_port = bridge.get_host_port_ipv4(BRIDGE_RPC_PORT).await?;
        tracing::info!(time = ?start.elapsed(), grpc_port = validator_grpc_port, rpc_port, "CelestiaDevNode has started");
        Ok(Self { validator, bridge })
    }

    pub async fn validator_port_ipv4(&self) -> anyhow::Result<u16> {
        self.validator
            .get_host_port_ipv4(VALIDATOR_GRPC_PORT)
            .await
            .context("failed to resolve validator host port")
    }

    pub async fn bridge_port_ipv4(&self) -> anyhow::Result<u16> {
        self.bridge
            .get_host_port_ipv4(BRIDGE_RPC_PORT)
            .await
            .context("failed to resolve bridge host port")
    }

    async fn run_validator_command(&self, command: Vec<String>) -> anyhow::Result<String> {
        const EXIT_POLL_ATTEMPTS: usize = 300;
        const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

        let mut exec = self
            .validator
            .exec(ExecCommand::new(command.clone()))
            .await
            .with_context(|| format!("failed to execute validator command: {command:?}"))?;

        let mut exit_code = exec
            .exit_code()
            .await
            .context("failed to obtain validator command exit code")?;

        if exit_code.is_none() {
            for _ in 0..EXIT_POLL_ATTEMPTS {
                sleep(EXIT_POLL_INTERVAL).await;
                exit_code = exec
                    .exit_code()
                    .await
                    .context("failed to obtain validator command exit code")?;
                if exit_code.is_some() {
                    break;
                }
            }
        }

        let exit_code =
            exit_code.ok_or_else(|| anyhow!("validator command did not finish within timeout"))?;

        let stdout_bytes = exec
            .stdout_to_vec()
            .await
            .context("failed to capture validator command stdout")?;
        let stdout =
            String::from_utf8(stdout_bytes).context("command output is not valid UTF-8")?;
        let stdout_trimmed = stdout.trim().to_owned();

        if exit_code != 0 {
            let stderr_bytes = exec
                .stderr_to_vec()
                .await
                .context("failed to capture validator command stderr")?;
            let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
            return Err(anyhow!(
                "validator command {:?} failed with exit code {exit_code}: {stderr}",
                command
            ));
        }

        Ok(stdout_trimmed)
    }

    /// By default, celestia validator will pre-fund 10 keys. Index starts from 0.
    pub async fn export_signer_key(&self, key_index: u8) -> anyhow::Result<String> {
        let key_name = format!("bridge-{key_index}");
        let command = vec![
            "sh".to_owned(),
            "-lc".to_owned(),
            format!(
                "yes | celestia-appd keys export --unsafe --unarmored-hex \"{key_name}\" --keyring-backend \"test\""
            ),
        ];

        let key = self
            .run_validator_command(command)
            .await
            .context("failed to export signer key")?;

        if key.is_empty() {
            return Err(anyhow!("validator key export returned empty output"));
        }

        Ok(key)
    }

    pub async fn get_signer_address(&self, key_index: u8) -> anyhow::Result<CelestiaAddress> {
        let key_name = format!("bridge-{key_index}");
        let command = vec![
            "sh".to_owned(),
            "-lc".to_owned(),
            format!(
                "celestia-appd keys show \"{key_name}\" --keyring-backend \"test\" --output=json | jq -r '.address'"
            ),
        ];

        let address = self
            .run_validator_command(command)
            .await
            .context("failed to query signer address")?;

        if address.is_empty() {
            return Err(anyhow!(
                "validator key show returned empty address for key {key_name}"
            ));
        }

        CelestiaAddress::from_str(&address).context("failed to parse signer address")
    }

    // Config for sequencer 0.
    pub async fn get_config(&self) -> anyhow::Result<CelestiaConfig> {
        let rpc_url = format!("ws://127.0.0.1:{}", self.bridge_port_ipv4().await?);
        let grpc_url = format!("http://127.0.0.1:{}", self.validator_port_ipv4().await?);

        let key_0 = self.export_signer_key(0).await?;
        let address_0 = self.get_signer_address(0).await?;
        tracing::info!(?address_0, ?key_0, "Celestia signer credentials ready");
        Ok(CelestiaConfig {
            rpc_url,
            rpc_auth_token: None,
            grpc_url: Some(grpc_url),
            grpc_auth_token: None,
            signer_private_key: Some(key_0),
            request_timeout_secs: default_request_timeout_seconds(),
            api_request_timeout_secs: default_api_request_timeout_secs(),
            tx_status_polling_millis: default_tx_status_polling_millis(),
            background_stat_polling_interval_secs: default_background_stat_polling_interval_secs(),
            safe_lead_time_ms: default_safe_lead_time_ms(),
            tx_priority: default_tx_priority(),
            backoff_min_delay_ms: default_min_delay_ms(),
            backoff_max_delay_ms: default_max_delay_ms(),
            backoff_max_times: default_max_times(),
            backoff_factor: default_factor(),
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_service_starts() -> anyhow::Result<()> {
    // sov_test_utils::logging::initialize_or_change_logging_with_filter(
    //     "debug,bollard=info,h2=warn,hyper=warn,jsonrpsee=warn,sov_metrics=off,tower=warn,",
    // );
    let dev_node = CelestiaDevNode::start().await?;

    let config = dev_node.get_config().await?;
    tracing::info!("CONFIG: {:?}", config);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    let da_service =
        CelestiaService::new(config, crate::test_helper::ROLLUP_PARAMS_DEV, shutdown_rx).await;
    let signer = da_service.get_signer().await;
    assert_eq!(signer, Some(dev_node.get_signer_address(0).await?));
    let header_1 = da_service.get_head_block_header().await?;
    let blob = vec![0, 1, 2, 3, 4];
    let _result = da_service.send_transaction(&blob).await.await??;
    let header_2 = da_service.get_head_block_header().await?;
    assert!(header_2.height() > header_1.height());

    for h in header_1.height()..=header_2.height() {
        let block = da_service.get_block_at(h).await?;
        let blobs = da_service.extract_relevant_blobs(&block);
        let batch_blobs = blobs.batch_blobs.len();
        tracing::info!(height=%h, %batch_blobs, "Got block data! ======");
        for blob in blobs.batch_blobs {
            let blob_signer = blob.sender;
            tracing::info!("BLOB {} SENDER {blob_signer}", blob.hash);
        }
    }

    Ok(())
}
