mod db_operations;
mod leader_election;

use std::net::SocketAddr;

use super::*;
use sov_full_node_configs::sequencer::{ConfiguredNodeRole, LeaderElectionConfig};

use sov_test_utils::postgres::{
    config_from_postgres_container, create_postgres_container, ContainerAsync, CreatePostgresError,
    Postgres,
};

pub(super) async fn setup_test_postgres() -> Option<ContainerAsync<Postgres>> {
    match create_postgres_container().await {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => None,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    }
}

pub(super) struct DB {
    pub(super) backend: PostgresBackend,
    pub(super) node_id: String,
    pub(super) node_address: String,
    election_config: LeaderElectionConfig,
}

impl AsRef<PostgresBackend> for DB {
    fn as_ref(&self) -> &PostgresBackend {
        &self.backend
    }
}

impl AsMut<PostgresBackend> for DB {
    fn as_mut(&mut self) -> &mut PostgresBackend {
        &mut self.backend
    }
}

impl DB {
    pub(super) async fn new(
        postgres: &ContainerAsync<Postgres>,
        node_id: String,
        node_role: ConfiguredNodeRole,
    ) -> Self {
        let election_config = LeaderElectionConfig {
            leader_timeout_millis: 100_000,
            grace_period_millis: 100_000,
            heartbeat_interval_millis: 100,
        };
        let postgres_config = config_from_postgres_container(postgres, node_id.clone(), node_role)
            .await
            .unwrap();
        let bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let node_address = node_address(bind_addr).unwrap();
        let backend = PostgresBackend::connect(&postgres_config, bind_addr)
            .await
            .unwrap();

        Self {
            node_address,
            backend,
            node_id,
            election_config,
        }
    }

    /// Overrides both leader_timeout and grace_period for testing.
    /// Both must be bypassed together when testing leadership takeover.
    pub(super) fn override_leader_timeout(&mut self, leader_timeout: Duration) {
        let millis = leader_timeout.as_millis() as u64;
        self.election_config.leader_timeout_millis = millis;
        self.election_config.grace_period_millis = millis;
    }

    /// Overrides leader_timeout and grace_period independently for testing grace period behavior.
    pub(super) fn override_timeouts(&mut self, leader_timeout: Duration, grace_period: Duration) {
        self.election_config.leader_timeout_millis = leader_timeout.as_millis() as u64;
        self.election_config.grace_period_millis = grace_period.as_millis() as u64;
    }

    pub(super) async fn maybe_update_leader(&self) -> Option<SequencerLeader> {
        let mut tx = self.backend.pool.begin().await.unwrap();
        let result = self
            .backend
            .try_update_leader_inner(
                &mut tx,
                self.election_config.leader_timeout(),
                self.election_config.grace_period(),
            )
            .await
            .unwrap();

        self.backend
            .upsert_node_registration_inner(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        result
    }

    pub(super) async fn get_sequencer_leader(&self) -> Result<Option<String>, sqlx::Error> {
        let mut tx = self.backend.pool.begin().await?;
        let res = self.backend.get_sequencer_leader_inner(&mut tx).await?;
        tx.commit().await?;
        Ok(res)
    }
}
