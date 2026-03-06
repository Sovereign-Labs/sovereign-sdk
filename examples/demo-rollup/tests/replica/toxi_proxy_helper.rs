use super::*;
use sov_test_utils::sov_toxi_proxi_image::ToxiProxySetup;

pub(crate) struct NodeTestSetup {
    postgres: Arc<PostgresData>,
    da_shutdown: watch::Sender<()>,
    da_addr: SocketAddr,
    toxiproxy_setup: ToxiProxySetup,
    direct_postgres_connection_string: String,
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

        let toxiproxy_setup = ToxiProxySetup::start_for_postgres_and_da(
            &direct_postgres_connection_string,
            da_container_addr.port(),
        )
        .await;

        Some(Self {
            postgres,
            da_shutdown,
            da_addr,
            toxiproxy_setup,
            direct_postgres_connection_string,
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
                self.toxiproxy_setup.proxied_da_addr(),
                proxied_node_id,
                self.toxiproxy_setup
                    .proxied_postgres_connection_string()
                    .to_string(),
                SequencerRole::PgSyncReplica,
            )
            .await;

        (leader_direct_node, replica_proxied_node)
    }

    /// Simulates or heals a Postgres partition by toggling the postgres proxy.
    pub(crate) async fn set_postgres_partition(&self, partitioned: bool) {
        self.toxiproxy_setup
            .set_postgres_partition(partitioned)
            .await;
    }

    /// Enables or disables high latency on the replica's DA traffic.
    pub(crate) async fn set_replica_da_slow(&self, slow: bool) {
        self.toxiproxy_setup.set_replica_da_slow(slow).await;
    }

    /// Shuts down both rollups, stops the DA producer, and drops the toxiproxy container.
    pub(crate) async fn shutdown(self, leader: TestRollup<Rollup>, replica: TestRollup<Rollup>) {
        let _ = replica.shutdown().await;
        let _ = leader.shutdown().await;
        let _ = self.da_shutdown.send(());
        self.toxiproxy_setup.shutdown();
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
}
