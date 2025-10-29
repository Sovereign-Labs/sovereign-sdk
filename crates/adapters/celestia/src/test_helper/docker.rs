#![allow(dead_code)]

use std::borrow::Cow;
use std::time::{Duration, Instant};

use crate::config::{
    default_factor, default_max_delay_ms, default_max_times, default_min_delay_ms,
    default_request_timeout_seconds, default_safe_lead_time_ms,
};
use crate::{CelestiaConfig, CelestiaService};
use anyhow::{anyhow, Context};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use testcontainers::core::{ExecCommand, Host, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image, ImageExt};
use tokio::time::sleep;
use uuid::Uuid;

const VALIDATOR_IMAGE: &str = "validator";
const BRIDGE_IMAGE: &str = "bridge";
const VALIDATOR_GRPC_PORT: u16 = 9090;
const BRIDGE_RPC_PORT: u16 = 26658;

pub struct CelestiaValidator;

impl Image for CelestiaValidator {
    fn name(&self) -> &str {
        VALIDATOR_IMAGE
    }

    fn tag(&self) -> &str {
        "latest"
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::healthcheck()]
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
        "latest"
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

pub struct CelestiaTestServer {
    validator: ContainerAsync<CelestiaValidator>,
    bridge: ContainerAsync<CelestiaBridge>,
}

impl CelestiaTestServer {
    pub async fn start() -> anyhow::Result<Self> {
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

        let server = Self { validator, bridge };
        server
            .wait_for_bridge_connectivity()
            .await
            .context("bridge node failed to reach validator")?;

        Ok(server)
    }

    pub async fn validator_port_ipv4(&self, internal_port: u16) -> anyhow::Result<u16> {
        self.validator
            .get_host_port_ipv4(internal_port)
            .await
            .context("failed to resolve validator host port")
    }

    pub async fn bridge_port_ipv4(&self, internal_port: u16) -> anyhow::Result<u16> {
        self.bridge
            .get_host_port_ipv4(internal_port)
            .await
            .context("failed to resolve bridge host port")
    }

    // TODO: Do we need that??
    async fn wait_for_bridge_connectivity(&self) -> anyhow::Result<()> {
        let timeout = Duration::from_secs(60);
        let poll_interval = Duration::from_millis(500);
        let deadline = Instant::now() + timeout;
        let mut last_error: Option<anyhow::Error> = None;
        let mut attempt = 0u32;

        while Instant::now() < deadline {
            attempt += 1;
            match self
                .bridge
                .exec(ExecCommand::new([
                    "grpcurl",
                    "-plaintext",
                    "validator:9090",
                    "list",
                ]))
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        attempts = attempt,
                        "Bridge confirmed connectivity with validator"
                    );
                    return Ok(());
                }
                Err(err) => {
                    let error: anyhow::Error = err.into();
                    tracing::debug!(
                        attempts = attempt,
                        error = %error,
                        "Bridge not ready yet; retrying"
                    );
                    last_error = Some(error);
                    sleep(poll_interval).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("grpcurl never succeeded")))
            .context("bridge connectivity check timed out")
    }

    pub async fn export_signer_key(&self, key_index: u8) -> anyhow::Result<String> {
        // Move those to sov-test utils at some point
        const EXIT_POLL_ATTEMPTS: usize = 300;
        const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

        let key_name = format!("bridge-{key_index}");
        let command = [
            "sh",
            "-lc",
            &format!(
                "yes | celestia-appd keys export --unsafe --unarmored-hex \"{key_name}\" --keyring-backend \"test\""
            ),
        ];

        let mut exec = self
            .validator
            .exec(ExecCommand::new(command))
            .await
            .context("failed to execute key export command")?;

        let mut exit_code = exec
            .exit_code()
            .await
            .context("failed to obtain export command exit code")?;

        if exit_code.is_none() {
            for _ in 0..EXIT_POLL_ATTEMPTS {
                sleep(EXIT_POLL_INTERVAL).await;
                exit_code = exec
                    .exit_code()
                    .await
                    .context("failed to obtain export command exit code")?;
                if exit_code.is_some() {
                    break;
                }
            }
        }

        let exit_code = exit_code
            .ok_or_else(|| anyhow!("validator key export did not finish within timeout"))?;

        if exit_code != 0 {
            let stderr = String::from_utf8_lossy(
                &exec
                    .stderr_to_vec()
                    .await
                    .context("failed to capture export stderr")?,
            )
            .into_owned();
            return Err(anyhow!(
                "validator key export failed with exit code {exit_code}: {stderr}"
            ));
        }

        let stdout_bytes = exec
            .stdout_to_vec()
            .await
            .context("failed to capture export stdout")?;
        let stdout = String::from_utf8(stdout_bytes).context("export output is not valid UTF-8")?;
        let key = stdout.trim().to_string();

        if key.is_empty() {
            return Err(anyhow!("validator key export returned empty output"));
        }

        Ok(key)
    }

    pub async fn get_config(&self) -> anyhow::Result<CelestiaConfig> {
        let rpc_url = format!(
            "ws://127.0.0.1:{}",
            self.bridge_port_ipv4(BRIDGE_RPC_PORT).await?
        );
        let grpc_url = format!(
            "http://127.0.0.1:{}",
            self.validator_port_ipv4(VALIDATOR_GRPC_PORT).await?
        );

        let key_0 = self.export_signer_key(0).await?;
        tracing::info!(?key_0, "Keys, baby!");
        Ok(CelestiaConfig {
            rpc_url,
            rpc_auth_token: None,
            grpc_url: Some(grpc_url),
            grpc_auth_token: None,
            signer_private_key: Some(key_0),
            request_timeout_secs: default_request_timeout_seconds(),
            safe_lead_time_ms: default_safe_lead_time_ms(),
            tx_priority: None,
            backoff_min_delay_ms: default_min_delay_ms(),
            backoff_max_delay_ms: default_max_delay_ms(),
            backoff_max_times: default_max_times(),
            backoff_factor: default_factor(),
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_service_starts() -> anyhow::Result<()> {
    sov_test_utils::logging::initialize_or_change_logging_with_filter(
        "debug,bollard=info,h2=warn,hyper=warn,jsonrpsee=warn,sov_metrics=off,tower=warn,",
    );

    let server = CelestiaTestServer::start().await?;
    let validator_port = server
        .validator_port_ipv4(VALIDATOR_GRPC_PORT)
        .await
        .context("failed to query validator port after startup")?;
    let bridge_port = server
        .bridge_port_ipv4(BRIDGE_RPC_PORT)
        .await
        .context("failed to query bridge port after startup")?;

    tracing::info!(
        validator_port,
        bridge_port,
        "Celestia test server is up and routing traffic"
    );

    let config = server.get_config().await?;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("CONFIG: {:?}", config);
    let da_service = CelestiaService::new(config, crate::test_helper::ROLLUP_PARAMS_DEV).await;
    let signer = da_service.get_signer().await;
    tracing::info!("DA SERVICE HAS BEEN BUILT WITH SIGNER: {}", signer);
    let header_1 = da_service.get_head_block_header().await?;
    tracing::info!("HEAD 1: {}", header_1.display());
    // tokio::time::sleep(std::time::Duration::from_secs(12)).await;
    let blob = vec![0, 1, 2, 3, 4];
    let result = da_service.send_transaction(&blob).await.await??;
    tracing::info!("Result: {:?}", result);
    let header_2 = da_service.get_head_block_header().await?;
    tracing::info!("HEAD 2: {}", header_2.display());

    for h in header_1.height()..=header_2.height() {
        let block = da_service.get_block_at(h).await?;
        let blobs = da_service.extract_relevant_blobs(&block);
        let batch_blobs = blobs.batch_blobs.len();
        tracing::info!(height=%h, %batch_blobs, "Got block data! ======");
        for blob in blobs.batch_blobs {
            let blob_signer = blob.sender;
            tracing::info!("BLOB {} SENDER {}", blob.hash, blob_signer)
        }
    }

    Ok(())
}
