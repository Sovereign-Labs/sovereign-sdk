use sov_sequencer::preferred::PostgresConfig;
use std::borrow::Cow;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image};

/// A Docker image for PostgreSQL.
#[derive(Debug, Clone, Default)]
pub struct PostgresImage;

impl Image for PostgresImage {
    fn name(&self) -> &str {
        "postgres"
    }

    fn tag(&self) -> &str {
        "17-alpine"
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        // See <https://github.com/testcontainers/testcontainers-rs-modules-community/issues/158>.
        vec![
            WaitFor::message_on_stderr("database system is ready to accept connections"),
            WaitFor::message_on_stdout("database system is ready to accept connections"),
        ]
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        [
            ("POSTGRES_DB", "postgres"),
            ("POSTGRES_USER", "postgres"),
            ("POSTGRES_PASSWORD", "postgres"),
        ]
    }
}

#[derive(Debug, thiserror::Error)]
/// Error indicating problems when creating a Postgres container.
pub enum CreatePostgresError {
    #[error("Docker is not supported on this platform")]
    /// Docker is not supported on this platform
    DockerNotSupported,

    #[error("Failed to create a Docker container: {0}")]
    /// Failed to create a Docker container, maybe the Docker daemon is not running.
    DockerError(#[from] anyhow::Error),
}

/// Creates a container with a PostgreSQL database.
pub async fn create_postgres_container(
) -> Result<ContainerAsync<PostgresImage>, CreatePostgresError> {
    if should_skip_postgres() {
        return Err(CreatePostgresError::DockerNotSupported);
    }

    let img = PostgresImage
        .start()
        .await
        .map_err(|e| CreatePostgresError::DockerError(e.into()))?;
    Ok(img)
}

fn should_skip_postgres() -> bool {
    // We skip all docker (i.e. postgres) tests on our dev server due to firewall false positives
    // bricking the machine.
    // The dev machine has 96 threads, which we detect to disable postgres. Currently no dev or CI
    // setup uses a machine of exactly this size, though if this ever changes this will cause
    // false positives.
    const DEV_SERVER_CPUS: usize = 96;

    if num_cpus::get() == DEV_SERVER_CPUS {
        return true;
    }

    if std::env::var("SOV_TEST_SKIP_DOCKER") == Ok("1".to_string()) {
        return true;
    }

    false
}

/// Returns the connection string for the PostgreSQL.
pub async fn connection_string_from_postgres_container(
    container: &ContainerAsync<PostgresImage>,
) -> anyhow::Result<String> {
    let postgres_connection_string = format!(
        "postgres://postgres:postgres@{}:{}",
        container.get_host().await?,
        container.get_host_port_ipv4(5432).await?
    );

    Ok(postgres_connection_string)
}

/// Returns the connection string for the PostgreSQL.
pub async fn config_from_postgres_container(
    container: &ContainerAsync<PostgresImage>,
    node_id: String,
) -> anyhow::Result<PostgresConfig> {
    let postgres_connection_string = connection_string_from_postgres_container(container).await?;
    Ok(PostgresConfig {
        postgres_connection_string,
        node_id,
    })
}
