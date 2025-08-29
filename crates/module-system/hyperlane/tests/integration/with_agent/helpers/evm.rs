use crate::with_agent::helpers::{EvmLog, ANVIL_ACCOUNTS};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sov_hyperlane_integration::EthAddress;
use sov_modules_api::Amount;
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::anvil::AnvilNode;

const ANVIL_PORT: u16 = 8545;
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
            .with_cmd([
                // TODO: Do we really need that? It looks like AnvilNode handles that by default.
                "--host",
                "0.0.0.0",
                "--port",
                &ANVIL_PORT.to_string(),
            ])
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

    // TODO: why something goes via cast send and other via RPC
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
                ANVIL_ACCOUNTS[0].1,
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
}

#[derive(Debug, Deserialize)]
struct CallOutput {
    logs: Vec<EvmLog>,
}
