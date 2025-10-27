use std::net::SocketAddr;
use std::num::NonZero;

use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::config::{default_request_timeout_seconds, CelestiaConfig};
use crate::da_service::CelestiaService;
use crate::verifier::RollupParams;

#[allow(dead_code)]
pub struct MockCelestiaNode {
    pub rpc_server: MockServer,
    pub grpc_server: tonic::transport::Server,
}

use super::keys::credentials_1;

// Mock gRPC server - just for tests that don't actually submit blobs
pub mod grpc {
    use super::*;

    #[allow(dead_code)]
    pub async fn start_mock_grpc_server() -> anyhow::Result<SocketAddr> {
        // For now, just return a dummy address
        // The actual gRPC calls will fail, but that's okay for tests that don't use them
        Ok("127.0.0.1:0".parse().unwrap())
    }
}

pub struct RpcIdEchoResponder {
    pub response_result: Value,
}

impl Respond for RpcIdEchoResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let request_body_json: Result<Value, _> = serde_json::from_slice(&request.body);

        let response_id = match request_body_json {
            Ok(json_value) => json_value.get("id").cloned().unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };

        ResponseTemplate::new(200).set_body_json(json!({
            "id": response_id,
            "jsonrpc": "2.0",
            "result": self.response_result
        }))
    }
}

pub async fn setup_test_service(
    timeout_sec: Option<u64>,
    params: RollupParams,
) -> (MockServer, CelestiaConfig, CelestiaService) {
    // Start a background HTTP server on a random local port
    let mock_server = MockServer::start().await;

    // Start mock gRPC server
    let grpc_addr = grpc::start_mock_grpc_server()
        .await
        .expect("Failed to start mock gRPC server");

    let super::keys::TestCredentials {
        private_key_hex,
        address,
    } = credentials_1();

    // Mock header.NetworkHead endpoint (called during client initialization)
    // Use real header data from test files
    let header_data: Value = serde_json::from_str(include_str!(
        "../../test_data/block_mocha_no_shares/header.json"
    ))
    .expect("Failed to parse header.json");

    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(json!({
            "method": "header.NetworkHead"
        })))
        .respond_with(RpcIdEchoResponder {
            response_result: header_data,
        })
        .mount(&mock_server)
        .await;

    // Mock state.AccountAddress endpoint
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(json!({
            "method": "state.AccountAddress"
        })))
        .respond_with(RpcIdEchoResponder {
            response_result: json!(address.to_string()),
        })
        .mount(&mock_server)
        .await;

    // Add a catch-all mock to log what requests are being made
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(|req: &Request| {
            let body = String::from_utf8_lossy(&req.body);
            println!("Unhandled RPC request: {body}");
            ResponseTemplate::new(404)
        })
        .mount(&mock_server)
        .await;

    let timeout_sec = timeout_sec
        .map(|t| NonZero::new(t).unwrap())
        .unwrap_or_else(default_request_timeout_seconds);
    let mut config = CelestiaConfig::dev_config(&mock_server.uri());
    config.request_timeout_secs = timeout_sec;
    let grpc_url = format!("http://{grpc_addr}");
    println!("GRPC: {grpc_url}");
    config.signer_private_key = Some(private_key_hex);
    config.grpc_url = Some(grpc_url);

    let da_service = CelestiaService::new(config.clone(), params).await;

    (mock_server, config, da_service)
}
