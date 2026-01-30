mod db_operations;
mod leader_election;

use std::net::SocketAddr;

use super::*;
use sov_full_node_configs::sequencer::ConfiguredNodeRole;

pub(super) fn test_bind_addr(node_id: &str) -> SocketAddr {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node_id.hash(&mut hasher);
    let port = hasher.finish() as u16;
    SocketAddr::from(([127, 0, 0, 1], port))
}
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
    leader_timeout: Duration,
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
        let leader_timeout = Duration::from_millis(100_000);
        let postgres_config = config_from_postgres_container(postgres, node_id.clone(), node_role)
            .await
            .unwrap();
        let bind_addr = test_bind_addr(&node_id);
        let backend = PostgresBackend::connect(&postgres_config, bind_addr)
            .await
            .unwrap();

        Self {
            backend,
            node_id,
            leader_timeout,
        }
    }

    pub(super) fn override_leader_timeout(&mut self, leader_timeout: Duration) {
        self.leader_timeout = leader_timeout;
    }

    pub(super) async fn maybe_update_leader(&self) -> Option<SequencerLeader> {
        self.backend
            .heartbeat(Some(self.leader_timeout))
            .await
            .unwrap()
    }

    pub(super) async fn get_sequencer_leader(&self) -> Result<Option<String>, sqlx::Error> {
        let mut tx = self.backend.pool.begin().await?;
        let res = self.backend.get_sequencer_leader_inner(&mut tx).await?;
        tx.commit().await?;
        Ok(res)
    }
}
