use super::*;
use reqwest::StatusCode;
use serde_json::json;
use testcontainers::core::{Host, IntoContainerPort};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const TOXIPROXY_IMAGE: &str = "ghcr.io/shopify/toxiproxy";
const TOXIPROXY_TAG: &str = "2.12.0";
const TOXIPROXY_API_PORT: u16 = 8474;
const TOXIPROXY_POSTGRES_PORT: u16 = 8666;
const TOXIPROXY_PROXY_NAME: &str = "postgres_replica";
const TOXIC_NAME: &str = "postgres_latency";
const TOXIC_LATENCY_MS: u64 = 1500;

async fn start_db_elected_node_with_connection(
    da_addr: SocketAddr,
    postgres: Arc<PostgresData>,
    node_id: &str,
    postgres_connection_string: String,
) -> TestRollup<Rollup> {
    let genesis = test_genesis_source(OperatingMode::Operator);
    RollupBuilder::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{da_addr}"),
        },
        Some((postgres, node_id.into(), ConfiguredNodeRole::DbElected)),
    )
    .await
    .set_config(move |c| match &mut c.sequencer_config {
        SequencerKindConfig::Standard(_) => {
            panic!("Expected preferred sequencer config for DbElected node")
        }
        SequencerKindConfig::Preferred(p) => {
            p.num_cache_warmup_workers = 0;
            p.postgres_config
                .as_mut()
                .expect("DbElected node should have postgres config")
                .postgres_connection_string = postgres_connection_string;
        }
    })
    .start_test_rollup()
    .await
    .unwrap()
}

fn parse_postgres_port(connection_string: &str) -> u16 {
    reqwest::Url::parse(connection_string)
        .expect("Invalid postgres connection string")
        .port_or_known_default()
        .expect("Postgres connection string is missing a port")
}

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

async fn wait_for_toxiproxy_ready(client: &reqwest::Client, api_base_url: &str) {
    let version_url = format!("{api_base_url}/version");

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match client.get(&version_url).send().await {
                Ok(response) if response.status().is_success() => return,
                Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await
    .expect("Timed out waiting for toxiproxy to become ready");
}

async fn create_postgres_proxy(client: &reqwest::Client, api_base_url: &str, postgres_port: u16) {
    let create_proxy_body = json!({
        "name": TOXIPROXY_PROXY_NAME,
        "listen": format!("0.0.0.0:{TOXIPROXY_POSTGRES_PORT}"),
        "upstream": format!("host.docker.internal:{postgres_port}"),
    });

    let response = client
        .post(format!("{api_base_url}/proxies"))
        .header("content-type", "application/json")
        .body(create_proxy_body.to_string())
        .send()
        .await
        .expect("Failed to create toxiproxy postgres proxy");

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable body>".to_string());
        panic!("Failed to create toxiproxy postgres proxy: {status} {body}");
    }
}

async fn add_latency_toxic(client: &reqwest::Client, api_base_url: &str) {
    let toxic_body = json!({
        "name": TOXIC_NAME,
        "type": "latency",
        "stream": "upstream",
        "attributes": {
            "latency": TOXIC_LATENCY_MS,
            "jitter": 0,
        }
    });

    let response = client
        .post(format!(
            "{api_base_url}/proxies/{TOXIPROXY_PROXY_NAME}/toxics"
        ))
        .header("content-type", "application/json")
        .body(toxic_body.to_string())
        .send()
        .await
        .expect("Failed to add toxiproxy latency toxic");

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable body>".to_string());
        panic!("Failed to add toxiproxy latency toxic: {status} {body}");
    }
}

async fn remove_latency_toxic(client: &reqwest::Client, api_base_url: &str) {
    let response = client
        .delete(format!(
            "{api_base_url}/proxies/{TOXIPROXY_PROXY_NAME}/toxics/{TOXIC_NAME}"
        ))
        .send()
        .await
        .expect("Failed to remove toxiproxy latency toxic");

    let status = response.status();
    assert!(
        status.is_success() || status == StatusCode::NOT_FOUND,
        "Failed to remove toxiproxy latency toxic: {status}",
    );
}

async fn wait_for_role(node: &TestRollup<Rollup>, expected: SequencerRole) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let role = node.sequencer_role().await.unwrap();
            if role == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Timed out waiting for role {expected:?}"));
}

async fn start_toxiproxy_for_postgres(
    postgres_connection_string: &str,
) -> (
    ContainerAsync<GenericImage>,
    reqwest::Client,
    String,
    String,
) {
    let toxiproxy = GenericImage::new(TOXIPROXY_IMAGE, TOXIPROXY_TAG)
        .with_exposed_port(TOXIPROXY_API_PORT.tcp())
        .with_exposed_port(TOXIPROXY_POSTGRES_PORT.tcp())
        .with_host("host.docker.internal", Host::HostGateway)
        .start()
        .await
        .expect("Failed to start toxiproxy container");

    let toxiproxy_host = toxiproxy
        .get_host()
        .await
        .expect("Failed to get toxiproxy host")
        .to_string();
    let toxiproxy_api_port = toxiproxy
        .get_host_port_ipv4(TOXIPROXY_API_PORT.tcp())
        .await
        .expect("Failed to map toxiproxy API port");
    let toxiproxy_postgres_port = toxiproxy
        .get_host_port_ipv4(TOXIPROXY_POSTGRES_PORT.tcp())
        .await
        .expect("Failed to map toxiproxy postgres port");

    let client = reqwest::Client::new();
    let api_base_url = format!("http://{toxiproxy_host}:{toxiproxy_api_port}");
    wait_for_toxiproxy_ready(&client, &api_base_url).await;

    let postgres_port = parse_postgres_port(postgres_connection_string);
    create_postgres_proxy(&client, &api_base_url, postgres_port).await;

    let proxied_connection_string = build_proxy_connection_string(
        postgres_connection_string,
        &toxiproxy_host,
        toxiproxy_postgres_port,
    );

    (toxiproxy, client, api_base_url, proxied_connection_string)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_with_toxiproxy_postgres_latency() {
    let postgres = PostgresData::create_postgres().await;
    let postgres = match postgres {
        Ok(pg) => pg,
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let (_, da_shutdown, da_addr) = create_da_service_periodic().await;

    let (toxiproxy, toxiproxy_client, toxiproxy_api_url, proxied_connection_string) =
        start_toxiproxy_for_postgres(postgres.connection_string()).await;

    let direct_node = start_db_elected_node_with_connection(
        da_addr,
        postgres.clone(),
        "direct_node",
        postgres.connection_string().to_string(),
    )
    .await;
    direct_node.wait_for_sequencer_ready().await.unwrap();

    let proxied_node = start_db_elected_node_with_connection(
        da_addr,
        postgres.clone(),
        "proxied_node",
        proxied_connection_string,
    )
    .await;
    proxied_node.wait_for_sequencer_ready().await.unwrap();
    wait_for_role(&proxied_node, SequencerRole::PgSyncReplica).await;

    let (leader, replica) = establish_leader_and_replica(direct_node, proxied_node).await;
    let mut event_subscription = replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    let token_id = config_gas_token_id();
    let receiver_addr = random_address();
    let leader_client = leader.api_client().clone();

    send_transfers(
        0,
        2,
        read_private_key::<S>("tx_signer_private_key.json"),
        receiver_addr,
        leader_client.clone(),
    )
    .await;
    wait_for_all_events_with_timeout(Duration::from_secs(5), 2, &mut event_subscription).await;

    add_latency_toxic(&toxiproxy_client, &toxiproxy_api_url).await;

    send_transfers(
        2,
        1,
        read_private_key::<S>("tx_signer_private_key.json"),
        receiver_addr,
        leader_client,
    )
    .await;

    let event_arrived_quickly =
        tokio::time::timeout(Duration::from_millis(700), event_subscription.next()).await;
    assert!(
        event_arrived_quickly.is_err(),
        "Expected toxiproxy latency to delay PostgreSQL sync",
    );

    remove_latency_toxic(&toxiproxy_client, &toxiproxy_api_url).await;
    wait_for_all_events_with_timeout(Duration::from_secs(10), 1, &mut event_subscription).await;

    let receiver_balance = replica
        .client
        .get_balance::<S>(&receiver_addr, &token_id, None)
        .await
        .unwrap();
    assert_eq!(receiver_balance.0, AMOUNT * 3);

    let _ = replica.shutdown().await;
    let _ = leader.shutdown().await;
    let _ = da_shutdown.send(());
    drop(toxiproxy);
}
