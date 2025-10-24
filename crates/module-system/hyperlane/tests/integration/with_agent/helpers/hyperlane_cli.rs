use crate::with_agent::configs::{
    core_config, ethtest_metadata, sovtest_addresses, sovtest_metadata, warp_route_config,
};
use crate::with_agent::helpers::{parse_eth_addr, DEPLOYER_ACCOUNT, EVM_MAILBOX, RELAYER_ACCOUNT};
use sov_hyperlane_integration::EthAddress;
use sov_modules_api::HexHash;
use std::str::FromStr;
use testcontainers::core::Mount;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerRequest, GenericImage, ImageExt};

const IMAGE: &str = "ghcr.io/sovereign-labs/hyperlane-cli";
const TAG: &str = "sov-integration-3";

pub struct HyperlaneCliRunner {
    data: tempfile::TempDir,
}

impl HyperlaneCliRunner {
    pub fn new(rollup_port: u16, anvil_port: u16, host_address: &str) -> Self {
        tracing::debug!(
            rollup_port,
            anvil_port,
            host_address,
            "Initializing runner for hyperlane-cli"
        );
        let data = tempfile::tempdir().expect("failed to create tempdir for hyperlane-cli data");
        prepare_cli_data(data.path(), rollup_port, anvil_port, host_address);

        Self { data }
    }

    pub fn prepare_container(&self) -> ContainerRequest<GenericImage> {
        let data_path = self.data.path();
        let chains_dir = data_path.join(".hyperlane").join("chains");
        let configs_dir = data_path.join("configs");

        let mut hyperlane_cli_image = GenericImage::new(IMAGE, TAG)
            // TODO: Move this to optional parameter
            .with_env_var("HYP_KEY", DEPLOYER_ACCOUNT.1)
            .with_mount(Mount::bind_mount(
                chains_dir.to_string_lossy().to_string(),
                "/root/.hyperlane/chains",
            ))
            .with_mount(Mount::bind_mount(
                configs_dir.to_string_lossy().to_string(),
                "/root/configs",
            ));

        // The hyperlane CLI accesses GitHub APIs quite heavily for its GitHub-hosted
        // registry, this can cause rate limiting in CI jobs. Include the GitHub token
        // so we use authenticated requests to try to avoid this
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            hyperlane_cli_image = hyperlane_cli_image.with_env_var("GH_AUTH_TOKEN", token);
        }

        hyperlane_cli_image
    }

    /// Returns an address of evm test recipient, to which we can dispatch test messages.
    pub async fn deploy_core(&self) -> HexHash {
        let hyperlane_cli_image = self.prepare_container().with_cmd([
            "core",
            "deploy",
            "--registry",
            "/root/.hyperlane",
            "--disableProxy",
            "--config",
            "/root/configs/core-config.yaml",
            "--chain",
            "ethtest",
            "--yes",
        ]);
        let pretty_stdout = wait_till_container_exit(hyperlane_cli_image).await;

        let deployment_output = parse_deployments_map(&pretty_stdout);
        let test_recipient = deployment_output
            .get("testRecipient")
            .expect("Failed to find 'testRecipient' in stdout");

        let mailbox = deployment_output
            .get("mailbox")
            .expect("Failed to find 'mailbox:' in stdout");
        let mailbox = EthAddress::from_str(mailbox).expect("failed to parse mailbox");
        assert_eq!(
            mailbox, EVM_MAILBOX,
            "Predefined mailbox does not match, probably needs to be updated: {:?}",
            mailbox.0
        );
        tracing::info!(
            actual_mailbox = %mailbox,
            expected_mailbox = %EVM_MAILBOX,
            full_output = ?deployment_output,
            "Deployed core");

        parse_eth_addr(test_recipient)
    }

    pub async fn deploy_warp(&self) -> HexHash {
        let warp_config = warp_route_config();
        tracing::info!(warp_config, "warp route config");
        let configs_dir = self.data.path().join("configs");
        std::fs::write(configs_dir.join("warp-route-deployment.yaml"), warp_config)
            .expect("Failed to write warp-route-deployment config");

        let hyperlane_cli_image = self.prepare_container().with_cmd([
            "warp",
            "deploy",
            "--registry",
            "/root/.hyperlane",
            "--disableProxy",
            "--config",
            "/root/configs/warp-route-deployment.yaml",
            "--yes",
        ]);

        let stdout = wait_till_container_exit(hyperlane_cli_image).await;

        // parse ethtest route address from logs
        let ethtest_route = stdout
            .lines()
            .find(|line| line.contains("addressOrDenom"))
            .unwrap()
            .split("\"")
            .nth(1)
            .unwrap();

        let ethtest_route = parse_eth_addr(ethtest_route);
        tracing::info!(%ethtest_route, evm_mailbox = %EVM_MAILBOX, "deployed warp");
        ethtest_route
    }
}

/// Renders configs with proper endpoints.
/// Each chain has an endpoint on the provided host address and passed port.
fn prepare_cli_data(
    data_path: &std::path::Path,
    rollup_port: u16,
    anvil_port: u16,
    host_address: &str,
) {
    // Create directory structure
    let hyperlane_dir = data_path.join(".hyperlane");
    let chains_dir = hyperlane_dir.join("chains");
    let sovtest_dir = chains_dir.join("sovtest");
    let ethtest_dir = chains_dir.join("ethtest");
    let configs_dir = data_path.join("configs");

    std::fs::create_dir_all(&sovtest_dir).expect("Failed to create 'sovtest' directory");
    std::fs::create_dir_all(&ethtest_dir).expect("Failed to create 'ethtest' directory");
    std::fs::create_dir_all(&configs_dir).expect("Failed to create 'configs' directory");

    let sovtest_config = sovtest_metadata(rollup_port, host_address);
    let ethtest_config = ethtest_metadata(host_address, anvil_port);
    let core_config = core_config(RELAYER_ACCOUNT.0.parse().unwrap());
    let sov_addresses = sovtest_addresses();

    std::fs::write(sovtest_dir.join("metadata.yaml"), sovtest_config)
        .expect("Failed to write 'sovtest' metadata");
    std::fs::write(ethtest_dir.join("metadata.yaml"), ethtest_config)
        .expect("Failed to write 'ethtest' metadata");
    std::fs::write(configs_dir.join("core-config.yaml"), core_config)
        .expect("Failed to write core-config");
    std::fs::write(sovtest_dir.join("addresses.yaml"), sov_addresses)
        .expect("Failed to write 'sovtest' addresses");
}

// Waits for some time while hyperlane-cli exit with status code 0
async fn wait_till_container_exit(hyperlane_cli_image: ContainerRequest<GenericImage>) -> String {
    let container: testcontainers::ContainerAsync<GenericImage> = hyperlane_cli_image
        .start()
        .await
        .expect("Failed to start hyperlane-cli");

    let mut is_running = container
        .is_running()
        .await
        .expect("failed to get running status");
    // 600 * 100ms = 60_000ms = 60s
    for _ in 0..600 {
        is_running = container.is_running().await.unwrap();
        if !is_running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // Temporarily print logs on failure to help with debugging. This will allow us to debug even when `tracing` isn't enabled.
    if is_running {
        let logs = container
            .stdout_to_vec()
            .await
            .expect("failed to get stdout from hyperlane-cli container after timeout.");
        println!("hyperlane-cli hasn't completed on time: \n\n CONTAINER STDOUT REPRODUCED BELOW:\n{}\n\n----------- END OF STDOUT ----------- ", String::from_utf8_lossy(&logs));
        let err_logs = container
            .stderr_to_vec()
            .await
            .expect("failed to get stderr from hyperlane-cli container after timeout.");
        println!("hyperlane-cli hasn't completed on time: \n\n CONTAINER STDERR REPRODUCED BELOW :\n{}\n\n ----------- END OF STDERR ----------- ", String::from_utf8_lossy(&err_logs));
        panic!("hyperlane-cli hasn't completed on time");
    }
    let container_exit_code = container
        .exit_code()
        .await
        .expect("Failed to get exit code from hyperlane-cli container");

    let container_stdout = container
        .stdout_to_vec()
        .await
        .expect("failed to get stdout from hyperlane-cli container");
    let pretty_stdout = String::from_utf8_lossy(&container_stdout).to_string();

    if container_exit_code != Some(0) {
        let container_stderr = container
            .stderr_to_vec()
            .await
            .expect("failed to get stderr from hyperlane-cli container");
        panic!(
            "Failed to deploy hyperlane: \nstdout:\n {} \nstderr: {}",
            pretty_stdout,
            String::from_utf8_lossy(&container_stderr)
        );
    }

    pretty_stdout
}

// Creates hashmap from the stdout, which should be something like that:
// ✅ Core contract deployments complete:
//
//     staticMerkleRootMultisigIsmFactory: "0x5FbDB2315678afecb367f032d93F642f64180aa3"
//     staticMessageIdMultisigIsmFactory: "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
//     staticAggregationIsmFactory: "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
//     staticAggregationHookFactory: "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
//     domainRoutingIsmFactory: "0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9"
//     staticMerkleRootWeightedMultisigIsmFactory: "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"
//     staticMessageIdWeightedMultisigIsmFactory: "0x0165878A594ca255338adfa4d48449f69242Eb8F"
//     proxyAdmin: "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853"
//     mailbox: "0x8A791620dd6260079BF849Dc5567aDC3F2FdC318"
//     interchainAccountRouter: "0x9A676e781A523b5d0C0e43731313A708CB607508"
//     validatorAnnounce: "0x0B306BF915C4d645ff596e518fAf3F9669b97016"
//     testRecipient: "0x959922bE3CAee4b8Cd9a407cc3ac1C251C2007B1"
//     merkleTreeHook: "0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e"
pub fn parse_deployments_map(input: &str) -> std::collections::HashMap<String, String> {
    input
        .lines()
        .skip_while(|l| !l.contains("Core contract deployments complete:"))
        .skip(1)
        .skip_while(|l| l.trim().is_empty())
        .map_while(|line| {
            let t = line.trim();
            let (k, v) = t.split_once(':')?;
            let v = v.trim().trim_end_matches(',').trim_matches('"');
            let k = k.trim().to_string();
            Some((k, v.to_string()))
        })
        .collect()
}
