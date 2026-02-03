//! ToxiProxy utilities for network chaos testing.
//!
//! This module provides helpers for creating PostgreSQL containers with a ToxiProxy
//! in front of them, enabling network fault injection for testing failover scenarios.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres;
use toxiproxy_rust::client::Client as ToxiClient;
use toxiproxy_rust::proxy::ProxyPack;

use crate::postgres::{create_postgres_container, CreatePostgresError};

/// Default ToxiProxy image tag.
const TOXIPROXY_IMAGE: &str = "ghcr.io/shopify/toxiproxy";
const TOXIPROXY_TAG: &str = "2.12.0";

/// ToxiProxy API port.
const TOXIPROXY_API_PORT: u16 = 8474;

/// ToxiProxy listen port for the PostgreSQL proxy.
const TOXIPROXY_LISTEN_PORT: u16 = 5433;

/// Data structure holding both PostgreSQL and ToxiProxy containers,
/// providing methods to manipulate network conditions.
pub struct ToxiProxyPostgresData {
    /// The PostgreSQL container (wrapped in Mutex for interior mutability).
    postgres: Mutex<Option<ContainerAsync<Postgres>>>,
    /// The ToxiProxy container (wrapped in Mutex for interior mutability).
    toxiproxy: Mutex<Option<ContainerAsync<GenericImage>>>,
    /// Connection string through ToxiProxy (for testing failover).
    proxied_connection_string: String,
    /// Direct connection string to PostgreSQL (for verification).
    direct_connection_string: String,
    /// ToxiProxy API URL for managing proxies.
    toxiproxy_api_url: String,
    /// Name of the proxy in ToxiProxy.
    proxy_name: String,
    /// Cached ToxiProxy client.
    toxi_client: OnceLock<ToxiClient>,
}

impl ToxiProxyPostgresData {
    /// Creates a new setup with PostgreSQL behind ToxiProxy.
    ///
    /// Returns an error if Docker is not supported on the platform.
    pub async fn create() -> Result<Arc<Self>, CreatePostgresError> {
        // Start PostgreSQL container
        let postgres = create_postgres_container().await?;

        // Get PostgreSQL host and port
        let pg_host = postgres.get_host().await.map_err(|e| {
            CreatePostgresError::DockerError(anyhow::anyhow!("Failed to get postgres host: {e}"))
        })?;
        let pg_port = postgres.get_host_port_ipv4(5432).await.map_err(|e| {
            CreatePostgresError::DockerError(anyhow::anyhow!("Failed to get postgres port: {e}"))
        })?;

        // Build the direct connection string
        let direct_connection_string =
            format!("postgres://postgres:postgres@{pg_host}:{pg_port}?sslmode=disable");

        // Start ToxiProxy container
        let toxiproxy = GenericImage::new(TOXIPROXY_IMAGE, TOXIPROXY_TAG)
            .with_exposed_port(TOXIPROXY_API_PORT.into())
            .with_exposed_port(TOXIPROXY_LISTEN_PORT.into())
            .start()
            .await
            .map_err(|e| {
                CreatePostgresError::DockerError(anyhow::anyhow!(
                    "Failed to start toxiproxy container: {e}"
                ))
            })?;

        // Get ToxiProxy host and ports
        let toxi_host = toxiproxy.get_host().await.map_err(|e| {
            CreatePostgresError::DockerError(anyhow::anyhow!("Failed to get toxiproxy host: {e}"))
        })?;
        let toxi_api_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_API_PORT)
            .await
            .map_err(|e| {
                CreatePostgresError::DockerError(anyhow::anyhow!(
                    "Failed to get toxiproxy API port: {e}"
                ))
            })?;
        let toxi_listen_port = toxiproxy
            .get_host_port_ipv4(TOXIPROXY_LISTEN_PORT)
            .await
            .map_err(|e| {
                CreatePostgresError::DockerError(anyhow::anyhow!(
                    "Failed to get toxiproxy listen port: {e}"
                ))
            })?;

        let toxiproxy_api_url = format!("{toxi_host}:{toxi_api_port}");
        let proxied_connection_string =
            format!("postgres://postgres:postgres@{toxi_host}:{toxi_listen_port}?sslmode=disable");

        let proxy_name = "postgres_proxy".to_string();

        let data = Arc::new(Self {
            postgres: Mutex::new(Some(postgres)),
            toxiproxy: Mutex::new(Some(toxiproxy)),
            proxied_connection_string,
            direct_connection_string,
            toxiproxy_api_url,
            proxy_name,
            toxi_client: OnceLock::new(),
        });

        // Configure the proxy to forward to PostgreSQL
        data.setup_proxy(pg_host.to_string(), pg_port).await?;

        Ok(data)
    }

    /// Sets up the ToxiProxy to forward traffic to PostgreSQL.
    async fn setup_proxy(&self, pg_host: String, pg_port: u16) -> Result<(), CreatePostgresError> {
        let client = self.get_client();

        // The upstream address is from the perspective of the ToxiProxy container.
        // Since both containers are on the same Docker network via host networking,
        // we use the host's address and port.
        let upstream = format!("{pg_host}:{pg_port}");
        let listen = format!("0.0.0.0:{TOXIPROXY_LISTEN_PORT}");

        // Create the proxy with retries (ToxiProxy may take a moment to start)
        let mut attempts = 0;
        loop {
            let proxy_to_create = ProxyPack {
                name: self.proxy_name.clone(),
                listen: listen.clone(),
                upstream: upstream.clone(),
                enabled: true,
                toxics: vec![],
            };

            match client.populate(vec![proxy_to_create]) {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    if attempts >= 10 {
                        return Err(CreatePostgresError::DockerError(anyhow::anyhow!(
                            "Failed to create toxiproxy proxy after 10 attempts: {}",
                            e
                        )));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }

        Ok(())
    }

    fn get_client(&self) -> &ToxiClient {
        self.toxi_client
            .get_or_init(|| ToxiClient::new(&self.toxiproxy_api_url))
    }

    /// Returns the connection string that routes through ToxiProxy.
    /// Use this for testing failover scenarios.
    pub fn proxied_connection_string(&self) -> &str {
        &self.proxied_connection_string
    }

    /// Returns the direct connection string to PostgreSQL.
    /// Use this for verification or when you need a stable connection.
    pub fn direct_connection_string(&self) -> &str {
        &self.direct_connection_string
    }

    /// Disables the proxy, simulating a complete network failure.
    pub async fn disable_proxy(&self) -> anyhow::Result<()> {
        let client = self.get_client();
        client
            .find_proxy(&self.proxy_name)
            .map_err(|e| anyhow::anyhow!("Proxy not found: {}", e))?
            .disable()
            .map_err(|e| anyhow::anyhow!("Failed to disable proxy: {}", e))?;
        Ok(())
    }

    /// Re-enables the proxy, restoring network connectivity.
    pub async fn enable_proxy(&self) -> anyhow::Result<()> {
        let client = self.get_client();
        client
            .find_proxy(&self.proxy_name)
            .map_err(|e| anyhow::anyhow!("Proxy not found: {}", e))?
            .enable()
            .map_err(|e| anyhow::anyhow!("Failed to enable proxy: {}", e))?;
        Ok(())
    }

    /// Adds latency to all connections through the proxy.
    ///
    /// # Arguments
    /// * `latency_ms` - Latency to add in milliseconds
    /// * `jitter_ms` - Optional jitter in milliseconds (randomness added to latency)
    pub async fn add_latency(&self, latency_ms: u64, jitter_ms: u64) -> anyhow::Result<()> {
        let client = self.get_client();
        let proxy = client
            .find_proxy(&self.proxy_name)
            .map_err(|e| anyhow::anyhow!("Proxy not found: {}", e))?;

        // Add latency toxic for downstream direction
        proxy
            .with_latency("downstream".into(), latency_ms as u32, jitter_ms as u32, 1.0)
            .apply(|| {
                // This closure is immediately executed, but the toxic persists
                // until we explicitly reset
            })
            .map_err(|e| anyhow::anyhow!("Failed to add latency: {}", e))?;

        Ok(())
    }

    /// Removes all toxics from the proxy, restoring normal operation.
    pub async fn remove_all_toxics(&self) -> anyhow::Result<()> {
        let client = self.get_client();
        client
            .find_and_reset_proxy(&self.proxy_name)
            .map_err(|e| anyhow::anyhow!("Failed to reset proxy: {e}"))?;
        Ok(())
    }

    /// Resets the proxy to a clean state (enabled, no toxics).
    pub async fn reset(&self) -> anyhow::Result<()> {
        self.enable_proxy().await?;
        self.remove_all_toxics().await?;
        Ok(())
    }

    /// Stops and removes the containers.
    /// This method must be called before dropping in an async context to avoid panics.
    pub async fn stop(&self) {
        // Take ownership of containers to stop them properly
        let toxiproxy = self.toxiproxy.lock().unwrap().take();
        let postgres = self.postgres.lock().unwrap().take();

        // Stop toxiproxy first, then postgres
        if let Some(container) = toxiproxy {
            // Ignoring errors during cleanup
            let _ = container.stop().await;
        }
        if let Some(container) = postgres {
            // Ignoring errors during cleanup
            let _ = container.stop().await;
        }
    }
}

impl Drop for ToxiProxyPostgresData {
    fn drop(&mut self) {
        // If containers weren't explicitly stopped via stop(), we need to leak them
        // to avoid the "Cannot drop a runtime in a context where blocking is not allowed" panic.
        // This can happen if the test panics before shutdown() is called.
        let toxiproxy = self.toxiproxy.lock().unwrap().take();
        let postgres = self.postgres.lock().unwrap().take();

        if let Some(container) = toxiproxy {
            // Leak the container to avoid panic in async context
            std::mem::forget(container);
        }
        if let Some(container) = postgres {
            // Leak the container to avoid panic in async context
            std::mem::forget(container);
        }
    }
}
