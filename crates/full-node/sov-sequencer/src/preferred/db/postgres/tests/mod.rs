mod db_operations;
mod leader_election;

use super::*;
use sov_full_node_configs::sequencer::ConfiguredNodeRole;
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
        let postgres_config =
            config_from_postgres_container(postgres, node_id.clone(), node_role)
                .await
                .unwrap();
        let backend =
            PostgresBackend::connect_internal(&postgres_config, format!("{node_id}_address"))
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
            .try_update_leader_and_register_node(self.leader_timeout)
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
