use crate::docker::pull_image_with_retries;
use serde_json::json;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use testcontainers::core::wait::HttpWaitStrategy;
use testcontainers::core::{ContainerPort, Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image, ImageExt};

const TOXIPROXY_IMAGE: &str = "ghcr.io/shopify/toxiproxy";
const TOXIPROXY_TAG: &str = "2.12.0";
// These are container-internal toxiproxy ports.
// Testcontainers maps each exposed container port to an ephemeral free host
// port (see `get_host_port_ipv4` below), so hardcoding these values does not
// create host-port conflicts across parallel tests or local services.
const TOXIPROXY_API_PORT: u16 = 8474;
const TOXIPROXY_POSTGRES_PORT: u16 = 8666;
const TOXIPROXY_DA_PORT: u16 = 8667;
const TOXIPROXY_POSTGRES_PROXY_NAME: &str = "proxied_postgres";
const TOXIPROXY_DA_PROXY_NAME: &str = "proxied_da";
const TOXIPROXY_SLOW_DA_TOXIC_NAME: &str = "proxied_slow_da";
const TOXIPROXY_SLOW_DA_LATENCY_MS: u64 = 1_000_000;
const TOXIPROXY_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const TOXIPROXY_READY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct SovToxiProxiImage;

impl Image for SovToxiProxiImage {
    fn name(&self) -> &str {
        TOXIPROXY_IMAGE
    }

    fn tag(&self) -> &str {
        TOXIPROXY_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::http(
            HttpWaitStrategy::new("/version")
                .with_port(TOXIPROXY_API_PORT.tcp())
                .with_expected_status_code(200u16),
        )]
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        const EXPOSED_PORTS: [ContainerPort; 3] = [
            ContainerPort::Tcp(TOXIPROXY_API_PORT),
            ContainerPort::Tcp(TOXIPROXY_POSTGRES_PORT),
            ContainerPort::Tcp(TOXIPROXY_DA_PORT),
        ];
        &EXPOSED_PORTS
    }
}

/// Running toxiproxy setup with computed proxied endpoints for Postgres and DA.
pub struct ToxiProxySetup {
    container: ContainerAsync<SovToxiProxiImage>,
    client: reqwest::Client,
    api_base_url: String,
    proxied_da_addr: SocketAddr,
    proxied_postgres_connection_string: String,
}

impl ToxiProxySetup {
    /// Starts toxiproxy, creates Postgres/DA proxies, and returns proxied endpoints.
    pub async fn start_for_postgres_and_da(
        postgres_connection_string: &str,
        da_upstream_port: u16,
    ) -> Self {
        pull_image_with_retries(SovToxiProxiImage)
            .await
            .expect("Failed to pull toxiproxy image");

        let toxiproxy = SovToxiProxiImage
            .with_host("host.docker.internal", Host::HostGateway)
            .with_startup_timeout(TOXIPROXY_READY_TIMEOUT)
            .start()
            .await
            .expect("Failed to start toxiproxy container");

        let toxiproxy_host = toxiproxy
            .get_host()
            .await
            .expect("Failed to get toxiproxy host")
            .to_string();

        // Resolve dynamic host ports that Docker assigned for this container
        // instance. Callers must use these values instead of the fixed
        // container-internal constants above.
        let toxiproxy_api_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_API_PORT.tcp())
            .await
            .expect("Failed to map toxiproxy API port");

        let toxiproxy_postgres_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_POSTGRES_PORT.tcp())
            .await
            .expect("Failed to map toxiproxy postgres port");
        let toxiproxy_da_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_DA_PORT.tcp())
            .await
            .expect("Failed to map toxiproxy DA port");

        let client = reqwest::Client::builder()
            .timeout(TOXIPROXY_HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("Failed to build toxiproxy reqwest client");

        let api_base_url = format!("http://{toxiproxy_host}:{toxiproxy_api_port}");

        let postgres_port = Self::parse_postgres_port(postgres_connection_string);
        Self::create_postgres_proxy(&client, &api_base_url, postgres_port).await;
        Self::create_da_proxy(&client, &api_base_url, da_upstream_port).await;

        let proxied_postgres_connection_string = Self::build_proxy_connection_string(
            postgres_connection_string,
            &toxiproxy_host,
            toxiproxy_postgres_port,
        );
        let proxied_da_addr = SocketAddr::new(
            Self::resolve_proxy_host_ip(&toxiproxy_host),
            toxiproxy_da_port,
        );

        Self {
            container: toxiproxy,
            client,
            api_base_url,
            proxied_da_addr,
            proxied_postgres_connection_string,
        }
    }

    /// Simulates or heals a Postgres partition by toggling the postgres proxy.
    pub async fn set_postgres_partition(&self, partitioned: bool) {
        let update_proxy_body = json!({
            "enabled": !partitioned,
        });

        Self::post_json(
            &self.client,
            format!(
                "{}/proxies/{}",
                self.api_base_url, TOXIPROXY_POSTGRES_PROXY_NAME
            ),
            &update_proxy_body,
            "Failed to update toxiproxy proxy state",
        )
        .await;
    }

    /// Enables or disables high latency on the replica's DA traffic.
    pub async fn set_replica_da_slow(&self, slow: bool) {
        Self::set_proxy_latency(
            &self.client,
            &self.api_base_url,
            TOXIPROXY_DA_PROXY_NAME,
            TOXIPROXY_SLOW_DA_TOXIC_NAME,
            TOXIPROXY_SLOW_DA_LATENCY_MS,
            slow,
            "Failed to configure slow replica DA communication toxic",
        )
        .await;
    }

    /// Returns the DA endpoint that points at toxiproxy.
    pub fn proxied_da_addr(&self) -> SocketAddr {
        self.proxied_da_addr
    }

    /// Returns the Postgres connection string rewritten to point at toxiproxy.
    pub fn proxied_postgres_connection_string(&self) -> &str {
        &self.proxied_postgres_connection_string
    }

    /// Drops the managed toxiproxy container.
    pub fn shutdown(self) {
        drop(self.container);
    }

    /// Sends a JSON POST request and fails fast when the response is not successful.
    async fn post_json(
        client: &reqwest::Client,
        url: String,
        body: &impl serde::Serialize,
        send_error_context: &str,
    ) {
        let response = client
            .post(url)
            .json(body)
            .send()
            .await
            .expect(send_error_context);

        let status = response.status();
        if !status.is_success() {
            panic!("{send_error_context}: {status}");
        }
    }

    /// Idempotently enables or disables a latency toxic on a given toxiproxy proxy.
    async fn set_proxy_latency(
        client: &reqwest::Client,
        api_base_url: &str,
        proxy_name: &str,
        toxic_name: &str,
        latency_ms: u64,
        enabled: bool,
        error_context: &str,
    ) {
        let toxic_base_url = format!("{api_base_url}/proxies/{proxy_name}/toxics");
        let toxic_url = format!("{toxic_base_url}/{toxic_name}");

        // Reset to a clean state first so this method is idempotent.
        Self::delete_toxic_if_exists(client, &toxic_url).await;

        if !enabled {
            return;
        }

        Self::post_json(
            client,
            toxic_base_url,
            &json!({
                "name": toxic_name,
                "type": "latency",
                "stream": "downstream",
                "toxicity": 1,
                "attributes": {
                    "latency": latency_ms,
                    "jitter": 0,
                },
            }),
            error_context,
        )
        .await;
    }

    /// Extracts the Postgres port from a connection string.
    fn parse_postgres_port(connection_string: &str) -> u16 {
        reqwest::Url::parse(connection_string)
            .expect("Invalid postgres connection string")
            .port_or_known_default()
            .expect("Postgres connection string is missing a port")
    }

    /// Rewrites a Postgres connection string to point at a toxiproxy host/port.
    fn build_proxy_connection_string(
        source_connection_string: &str,
        proxy_host: &str,
        proxy_port: u16,
    ) -> String {
        let parsed = reqwest::Url::parse(source_connection_string)
            .expect("Invalid postgres connection string for proxying");
        let username = parsed.username();
        let password = parsed
            .password()
            .expect("Expected password in postgres connection string");
        let path = parsed.path();
        let query_suffix = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();

        format!("postgres://{username}:{password}@{proxy_host}:{proxy_port}{path}{query_suffix}")
    }

    /// Resolves a testcontainers host value to an IP for `SocketAddr` consumers.
    /// Prefers IPv4 because ports are mapped with `get_host_port_ipv4`.
    fn resolve_proxy_host_ip(proxy_host: &str) -> std::net::IpAddr {
        if let Ok(ip) = proxy_host.parse::<std::net::IpAddr>() {
            return ip;
        }

        let mut addrs = (proxy_host, 0)
            .to_socket_addrs()
            .expect("Failed to resolve toxiproxy host");

        addrs
            .find(|addr| addr.is_ipv4())
            .or_else(|| addrs.next())
            .map(|addr| addr.ip())
            .expect("toxiproxy host resolved to no addresses")
    }

    /// Creates the toxiproxy listener that forwards Postgres traffic to the host Postgres container.
    async fn create_postgres_proxy(
        client: &reqwest::Client,
        api_base_url: &str,
        postgres_port: u16,
    ) {
        let create_proxy_body = json!({
            "name": TOXIPROXY_POSTGRES_PROXY_NAME,
            "listen": format!("0.0.0.0:{TOXIPROXY_POSTGRES_PORT}"),
            "upstream": format!("host.docker.internal:{postgres_port}"),
        });

        Self::post_json(
            client,
            format!("{api_base_url}/proxies"),
            &create_proxy_body,
            "Failed to create toxiproxy postgres proxy",
        )
        .await;
    }

    /// Creates the toxiproxy listener that forwards DA traffic to the host DA service.
    async fn create_da_proxy(client: &reqwest::Client, api_base_url: &str, da_port: u16) {
        let create_proxy_body = json!({
            "name": TOXIPROXY_DA_PROXY_NAME,
            // The DA proxy listens on the container-internal port. Each test gets
            // its own toxiproxy container, so this fixed listen port is isolated.
            "listen": format!("0.0.0.0:{TOXIPROXY_DA_PORT}"),
            "upstream": format!("host.docker.internal:{da_port}"),
        });

        Self::post_json(
            client,
            format!("{api_base_url}/proxies"),
            &create_proxy_body,
            "Failed to create toxiproxy DA proxy",
        )
        .await;
    }

    /// Deletes a toxic if present, tolerating `404 Not Found`.
    async fn delete_toxic_if_exists(client: &reqwest::Client, toxic_url: &str) {
        let response = client
            .delete(toxic_url)
            .send()
            .await
            .expect("Failed to delete toxiproxy toxic");

        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            panic!("Failed to delete toxiproxy toxic: {status}");
        }
    }
}
