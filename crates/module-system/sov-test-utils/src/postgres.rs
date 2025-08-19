use std::borrow::Cow;
use std::path::Path;
use testcontainers::core::Mount;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image, ImageExt};
use tracing::debug;

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

/// Creates a container with a PostgreSQL database.
pub async fn create_postgres_container(
    dir: &Path,
) -> anyhow::Result<ContainerAsync<PostgresImage>> {
    let postgres_data_dir = dir.join("postgres_data");
    debug!(?postgres_data_dir, "Using Postgres data directory");
    std::fs::create_dir_all(&postgres_data_dir)?;

    Ok(PostgresImage
        .with_mount(Mount::bind_mount(
            postgres_data_dir.to_string_lossy(),
            "/var/lib/postgresql/data",
        ))
        .start()
        .await?)
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
