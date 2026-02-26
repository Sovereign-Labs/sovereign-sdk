use super::*;
use serde_json::json;
use std::net::ToSocketAddrs;
use testcontainers::core::{Host, IntoContainerPort};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const TOXIPROXY_IMAGE: &str = "ghcr.io/shopify/toxiproxy";
const TOXIPROXY_TAG: &str = "2.12.0";
// These are container-internal toxiproxy ports.
// Testcontainers maps each exposed container port to an ephemeral free host
// port (see `get_host_port_ipv4` below), so hardcoding these values does not
// create host-port conflicts across parallel tests or local services.
const TOXIPROXY_API_PORT: u16 = 8474;
const TOXIPROXY_POSTGRES_PORT: u16 = 8666;
const TOXIPROXY_DA_PORT: u16 = 8667;
const TOXIPROXY_POSTGRES_PROXY_NAME: &str = "postgres_replica";
const TOXIPROXY_DA_PROXY_NAME: &str = "da_replica";
const TOXIPROXY_SLOW_DA_TOXIC_NAME: &str = "slow_da_replica";
const TOXIPROXY_SLOW_DA_LATENCY_MS: u64 = 1_000_000;
const TOXIPROXY_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const TOXIPROXY_READY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct NodeTestSetup {
    postgres: Arc<PostgresData>,
    da_shutdown: watch::Sender<()>,
    da_addr: SocketAddr,
    toxiproxy: ContainerAsync<GenericImage>,
    toxiproxy_client: reqwest::Client,
    toxiproxy_api_url: String,
    proxied_da_addr: SocketAddr,
    direct_postgres_connection_string: String,
    proxied_postgres_connection_string: String,
}

impl NodeTestSetup {
    /// Builds a full replica test setup with Postgres, DA service, and toxiproxy.
    /// Returns `None` when Docker-backed Postgres is not available in this environment.
    pub(crate) async fn new() -> Option<Self> {
        let postgres = PostgresData::create_postgres().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let (da_service, da_shutdown, da_addr) = create_da_service_periodic().await;
        // `create_da_service_periodic` binds DA RPC to loopback for host-side clients.
        // Toxiproxy runs in a container and reaches the host through
        // `host.docker.internal`, which cannot reach loopback-only sockets on Linux.
        // Expose the same DA service on all interfaces for toxiproxy upstream.
        let da_container_addr = start_server(da_service.clone(), "0.0.0.0", 0)
            .await
            .expect("Failed to expose DA service for toxiproxy upstream");
        let direct_postgres_connection_string = postgres.connection_string().to_string();

        let (
            toxiproxy,
            toxiproxy_client,
            toxiproxy_api_url,
            proxied_da_addr,
            proxied_postgres_connection_string,
        ) = Self::start_toxiproxy_for_postgres_and_da(
            &direct_postgres_connection_string,
            da_container_addr.port(),
        )
        .await;

        Some(Self {
            postgres,
            da_shutdown,
            da_addr,
            toxiproxy,
            toxiproxy_client,
            toxiproxy_api_url,
            proxied_da_addr,
            direct_postgres_connection_string,
            proxied_postgres_connection_string,
        })
    }

    /// Starts a DB-elected leader node on direct connections and a replica through proxies.
    pub(crate) async fn start_db_elected_pair(
        &mut self,
        direct_node_id: &str,
        proxied_node_id: &str,
    ) -> (TestRollup<Rollup>, TestRollup<Rollup>) {
        let leader_direct_node = self
            .start_db_elected_node(
                self.da_addr,
                direct_node_id,
                self.direct_postgres_connection_string.clone(),
                SequencerRole::BatchProducer,
            )
            .await;

        let replica_proxied_node = self
            .start_db_elected_node(
                self.proxied_da_addr,
                proxied_node_id,
                self.proxied_postgres_connection_string.clone(),
                SequencerRole::PgSyncReplica,
            )
            .await;

        (leader_direct_node, replica_proxied_node)
    }

    /// Simulates or heals a Postgres partition by toggling the postgres proxy.
    pub(crate) async fn set_postgres_partition(&self, partitioned: bool) {
        let update_proxy_body = json!({
            "enabled": !partitioned,
        });

        post_json(
            &self.toxiproxy_client,
            format!(
                "{}/proxies/{}",
                self.toxiproxy_api_url, TOXIPROXY_POSTGRES_PROXY_NAME
            ),
            &update_proxy_body,
            "Failed to update toxiproxy proxy state",
        )
        .await;
    }

    /// Enables or disables high latency on the replica's DA traffic.
    pub(crate) async fn set_replica_da_slow(&self, slow: bool) {
        set_proxy_latency(
            &self.toxiproxy_client,
            &self.toxiproxy_api_url,
            TOXIPROXY_DA_PROXY_NAME,
            TOXIPROXY_SLOW_DA_TOXIC_NAME,
            TOXIPROXY_SLOW_DA_LATENCY_MS,
            slow,
            "Failed to configure slow replica DA communication toxic",
        )
        .await;
    }

    /// Shuts down both rollups, stops the DA producer, and drops the toxiproxy container.
    pub(crate) async fn shutdown(self, leader: TestRollup<Rollup>, replica: TestRollup<Rollup>) {
        let _ = replica.shutdown().await;
        let _ = leader.shutdown().await;
        let _ = self.da_shutdown.send(());
        drop(self.toxiproxy);
    }

    /// Starts one DB-elected node against the provided DA and Postgres endpoints and checks its role.
    async fn start_db_elected_node(
        &self,
        da_addr: SocketAddr,
        node_id: &str,
        postgres_connection_string: String,
        expected_role: SequencerRole,
    ) -> TestRollup<Rollup> {
        let node = Some((
            self.postgres.clone(),
            node_id.into(),
            ConfiguredNodeRole::DbElected,
        ));
        let node =
            start_rollup_with_connection_string(da_addr, node, Some(postgres_connection_string))
                .await;

        node.wait_for_sequencer_ready().await.unwrap();
        let role = node.sequencer_role().await.unwrap();
        assert_eq!(role, expected_role);
        node
    }

    /// Starts toxiproxy, creates Postgres/DA proxies, and returns proxied endpoints.
    async fn start_toxiproxy_for_postgres_and_da(
        postgres_connection_string: &str,
        da_upstream_port: u16,
    ) -> (
        ContainerAsync<GenericImage>,
        reqwest::Client,
        String,
        SocketAddr,
        String,
    ) {
        let toxiproxy = GenericImage::new(TOXIPROXY_IMAGE, TOXIPROXY_TAG)
            .with_exposed_port(TOXIPROXY_API_PORT.tcp())
            .with_exposed_port(TOXIPROXY_POSTGRES_PORT.tcp())
            .with_exposed_port(TOXIPROXY_DA_PORT.tcp())
            .with_host("host.docker.internal", Host::HostGateway)
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
        wait_for_toxiproxy_ready(&client, &api_base_url).await;

        let postgres_port = parse_postgres_port(postgres_connection_string);
        create_postgres_proxy(&client, &api_base_url, postgres_port).await;
        create_da_proxy(&client, &api_base_url, da_upstream_port).await;

        let proxied_connection_string = build_proxy_connection_string(
            postgres_connection_string,
            &toxiproxy_host,
            toxiproxy_postgres_port,
        );
        let proxied_da_addr =
            SocketAddr::new(resolve_proxy_host_ip(&toxiproxy_host), toxiproxy_da_port);

        (
            toxiproxy,
            client,
            api_base_url,
            proxied_da_addr,
            proxied_connection_string,
        )
    }
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

/// Polls toxiproxy's `/version` endpoint until it responds or the timeout is reached.
async fn wait_for_toxiproxy_ready(client: &reqwest::Client, api_base_url: &str) {
    let version_url = format!("{api_base_url}/version");
    let start = std::time::Instant::now();

    loop {
        match client.get(&version_url).send().await {
            Ok(response) if response.status().is_success() => return,
            Ok(_) | Err(_) => {
                if start.elapsed() >= TOXIPROXY_READY_TIMEOUT {
                    panic!("Timed out waiting for toxiproxy to become ready");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Creates the toxiproxy listener that forwards Postgres traffic to the host Postgres container.
async fn create_postgres_proxy(client: &reqwest::Client, api_base_url: &str, postgres_port: u16) {
    let create_proxy_body = json!({
        "name": TOXIPROXY_POSTGRES_PROXY_NAME,
        "listen": format!("0.0.0.0:{TOXIPROXY_POSTGRES_PORT}"),
        "upstream": format!("host.docker.internal:{postgres_port}"),
    });

    post_json(
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

    post_json(
        client,
        format!("{api_base_url}/proxies"),
        &create_proxy_body,
        "Failed to create toxiproxy DA proxy",
    )
    .await;
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
    delete_toxic_if_exists(client, &toxic_url).await;

    if !enabled {
        return;
    }

    post_json(
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
