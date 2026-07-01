use crate::docker::prepull_image_best_effort;
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

// Container-internal ports for the Celestia RPC proxy fleet. Each fleet proxy fronts the
// SAME Celestia bridge RPC upstream; a test configures the adapter with proxy 0 as the
// primary `rpc_url` and the rest as ordered fallback endpoints, then breaks individual
// proxies to drive and observe failover. The length of this pool is the max fleet size.
const TOXIPROXY_CELESTIA_RPC_PORTS: [u16; 3] = [8668, 8669, 8670];

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
        const EXPOSED_PORTS: [ContainerPort; 6] = [
            ContainerPort::Tcp(TOXIPROXY_API_PORT),
            ContainerPort::Tcp(TOXIPROXY_POSTGRES_PORT),
            ContainerPort::Tcp(TOXIPROXY_DA_PORT),
            ContainerPort::Tcp(TOXIPROXY_CELESTIA_RPC_PORTS[0]),
            ContainerPort::Tcp(TOXIPROXY_CELESTIA_RPC_PORTS[1]),
            ContainerPort::Tcp(TOXIPROXY_CELESTIA_RPC_PORTS[2]),
        ];
        &EXPOSED_PORTS
    }
}

/// Running toxiproxy setup with computed proxied endpoints for Postgres and DA.
pub struct ToxiProxySetup {
    container: ContainerAsync<SovToxiProxiImage>,
    client: reqwest::Client,
    api_base_url: String,
    proxied_da_addr: Option<SocketAddr>,
    proxied_postgres_connection_string: String,
}

impl ToxiProxySetup {
    /// Starts toxiproxy, creates Postgres/DA proxies, and returns proxied endpoints.
    pub async fn start_for_postgres_and_da(
        postgres_connection_string: &str,
        da_upstream_port: u16,
    ) -> Self {
        Self::start(postgres_connection_string, Some(da_upstream_port)).await
    }

    /// Starts toxiproxy with only a Postgres proxy.
    pub async fn start_for_postgres(postgres_connection_string: &str) -> Self {
        Self::start(postgres_connection_string, None).await
    }

    /// Starts toxiproxy with a Postgres proxy, plus a DA proxy when
    /// `da_upstream_port` is `Some`.
    async fn start(postgres_connection_string: &str, da_upstream_port: Option<u16>) -> Self {
        prepull_image_best_effort(SovToxiProxiImage).await;

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

        let client = reqwest::Client::builder()
            .timeout(TOXIPROXY_HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("Failed to build toxiproxy reqwest client");

        let api_base_url = format!("http://{toxiproxy_host}:{toxiproxy_api_port}");

        let postgres_port = Self::parse_postgres_port(postgres_connection_string);
        Self::create_postgres_proxy(&client, &api_base_url, postgres_port).await;

        let proxied_postgres_connection_string = Self::build_proxy_connection_string(
            postgres_connection_string,
            &toxiproxy_host,
            toxiproxy_postgres_port,
        );

        // The DA proxy is only created when the caller has DA traffic to route.
        let proxied_da_addr = match da_upstream_port {
            Some(da_upstream_port) => {
                Self::create_da_proxy(&client, &api_base_url, da_upstream_port).await;
                let toxiproxy_da_port = toxiproxy
                    .get_host_port_ipv4(TOXIPROXY_DA_PORT.tcp())
                    .await
                    .expect("Failed to map toxiproxy DA port");
                Some(SocketAddr::new(
                    Self::resolve_proxy_host_ip(&toxiproxy_host),
                    toxiproxy_da_port,
                ))
            }
            None => None,
        };

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
            .expect("DA proxy was not configured; create the setup with start_for_postgres_and_da")
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

/// Toxiproxy fronting a single Celestia bridge RPC upstream with a fleet of `N`
/// independently controllable proxies, all pointing at the same node. Index `0` is the
/// primary; indices `1..N` are ordered fallback endpoints.
///
/// This is the topology for testing RPC endpoint failover and switch-back: the adapter is
/// configured with proxy `0` as `rpc_url` and proxies `1..N` as fallback endpoints, then a
/// test breaks individual proxies (disable / reset / timeout) and tags them with latency to
/// drive and observe failover.
pub struct CelestiaRpcProxyFleet {
    container: ContainerAsync<SovToxiProxiImage>,
    client: reqwest::Client,
    api_base_url: String,
    host: String,
    /// Mapped host ports, indexed by endpoint (index 0 is the primary).
    host_ports: Vec<u16>,
    /// Toxiproxy proxy names, indexed by endpoint.
    proxy_names: Vec<String>,
}

impl CelestiaRpcProxyFleet {
    /// Starts toxiproxy and creates `num_endpoints` proxies that all forward to the
    /// Celestia bridge RPC published on the host at `bridge_rpc_host_port`.
    pub async fn start(bridge_rpc_host_port: u16, num_endpoints: usize) -> Self {
        assert!(
            (1..=TOXIPROXY_CELESTIA_RPC_PORTS.len()).contains(&num_endpoints),
            "num_endpoints must be in 1..={}",
            TOXIPROXY_CELESTIA_RPC_PORTS.len()
        );

        prepull_image_best_effort(SovToxiProxiImage).await;

        let toxiproxy = SovToxiProxiImage
            .with_host("host.docker.internal", Host::HostGateway)
            .with_startup_timeout(TOXIPROXY_READY_TIMEOUT)
            .start()
            .await
            .expect("Failed to start toxiproxy container");

        let host = toxiproxy
            .get_host()
            .await
            .expect("Failed to get toxiproxy host")
            .to_string();

        let api_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_API_PORT.tcp())
            .await
            .expect("Failed to map toxiproxy API port");

        let client = reqwest::Client::builder()
            .timeout(TOXIPROXY_HTTP_REQUEST_TIMEOUT)
            .build()
            .expect("Failed to build toxiproxy reqwest client");

        let api_base_url = format!("http://{host}:{api_port}");

        // Every proxy forwards to the same bridge RPC upstream on the host.
        let mut host_ports = Vec::with_capacity(num_endpoints);
        let mut proxy_names = Vec::with_capacity(num_endpoints);
        for (index, &container_port) in TOXIPROXY_CELESTIA_RPC_PORTS
            .iter()
            .take(num_endpoints)
            .enumerate()
        {
            let name = format!("celestia_rpc_{index}");
            Self::create_proxy(
                &client,
                &api_base_url,
                &name,
                container_port,
                bridge_rpc_host_port,
            )
            .await;
            let host_port = toxiproxy
                .get_host_port_ipv4(container_port.tcp())
                .await
                .expect("Failed to map celestia RPC proxy port");
            host_ports.push(host_port);
            proxy_names.push(name);
        }

        Self {
            container: toxiproxy,
            client,
            api_base_url,
            host,
            host_ports,
            proxy_names,
        }
    }

    /// The WebSocket URL for endpoint `index` (0 = primary, 1.. = fallbacks).
    pub fn rpc_ws_url(&self, index: usize) -> String {
        format!("ws://{}:{}", self.host, self.host_ports[index])
    }

    /// Enables or disables the proxy for endpoint `index`. Disabling force-closes live
    /// connections and refuses new ones, exactly like the node going away.
    pub async fn set_enabled(&self, index: usize, enabled: bool) {
        ToxiProxySetup::post_json(
            &self.client,
            format!("{}/proxies/{}", self.api_base_url, self.proxy_names[index]),
            &json!({ "enabled": enabled }),
            "Failed to update celestia rpc proxy state",
        )
        .await;
    }

    /// Adds (or, with `None`, removes) a downstream latency toxic on endpoint `index`. A
    /// persistent latency lets a test tell which endpoint is currently serving traffic by
    /// measuring request round-trip time.
    pub async fn set_latency(&self, index: usize, latency: Option<Duration>) {
        let (latency_ms, enabled) = match latency {
            Some(latency) => (latency.as_millis() as u64, true),
            None => (0, false),
        };
        ToxiProxySetup::set_proxy_latency(
            &self.client,
            &self.api_base_url,
            &self.proxy_names[index],
            &format!("{}_latency", self.proxy_names[index]),
            latency_ms,
            enabled,
            "Failed to configure celestia rpc latency toxic",
        )
        .await;
    }

    /// Enables or disables an immediate TCP RST (`reset_peer`) on endpoint `index`. The
    /// proxy stays enabled — this models a connection reset, not a refused connection.
    pub async fn set_reset_peer(&self, index: usize, enabled: bool) {
        self.set_break_toxic(index, "reset_peer", json!({ "timeout": 0 }), enabled)
            .await;
    }

    /// Makes endpoint `index` hang indefinitely: a `timeout` toxic with `timeout: 0` drops all
    /// data and never closes the connection, so requests exceed the client's per-call timeout
    /// while the socket stays open. Unlike [`Self::set_reset_peer`], there is no connection-level
    /// error — this exercises the client's *timeout-based* failover. Pass `false` to heal.
    pub async fn set_timeout(&self, index: usize, enabled: bool) {
        self.set_break_toxic(index, "timeout", json!({ "timeout": 0 }), enabled)
            .await;
    }

    /// Drops the managed toxiproxy container.
    pub fn shutdown(self) {
        drop(self.container);
    }

    /// Idempotently (re)creates or removes a connection-breaking toxic on endpoint
    /// `index`, applied on both the upstream and downstream streams so it trips on a new
    /// connection's handshake bytes as well as on an already-established idle connection.
    async fn set_break_toxic(
        &self,
        index: usize,
        toxic_type: &str,
        attributes: serde_json::Value,
        enabled: bool,
    ) {
        let toxic_base_url = format!(
            "{}/proxies/{}/toxics",
            self.api_base_url, self.proxy_names[index]
        );
        for stream in ["upstream", "downstream"] {
            let toxic_name = format!("{toxic_type}_{stream}");
            let toxic_url = format!("{toxic_base_url}/{toxic_name}");
            // Reset to a clean state first so this method is idempotent.
            ToxiProxySetup::delete_toxic_if_exists(&self.client, &toxic_url).await;
            if !enabled {
                continue;
            }
            ToxiProxySetup::post_json(
                &self.client,
                toxic_base_url.clone(),
                &json!({
                    "name": toxic_name,
                    "type": toxic_type,
                    "stream": stream,
                    "toxicity": 1,
                    "attributes": attributes,
                }),
                "Failed to configure celestia rpc break toxic",
            )
            .await;
        }
    }

    /// Creates a toxiproxy listener that forwards to the host's Celestia bridge RPC.
    async fn create_proxy(
        client: &reqwest::Client,
        api_base_url: &str,
        name: &str,
        listen_port: u16,
        upstream_host_port: u16,
    ) {
        let create_proxy_body = json!({
            "name": name,
            "listen": format!("0.0.0.0:{listen_port}"),
            "upstream": format!("host.docker.internal:{upstream_host_port}"),
        });

        ToxiProxySetup::post_json(
            client,
            format!("{api_base_url}/proxies"),
            &create_proxy_body,
            "Failed to create celestia rpc proxy",
        )
        .await;
    }
}
