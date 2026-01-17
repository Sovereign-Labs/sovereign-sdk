//! Leadership election and heartbeat tasks for DbElected nodes.
//!
//! DbElected nodes attempt to acquire leadership at startup. If successful, they
//! run as leaders with a heartbeat task. If not, they run as replicas while
//! continuously attempting to acquire leadership.

use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use sov_full_node_configs::sequencer::PostgresConfig;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use super::postgres::PostgresBackend;
use crate::preferred::exit_rollup;

/// How often replicas attempt to acquire leadership.
const ELECTION_INTERVAL: Duration = Duration::from_millis(200);

/// How often leaders refresh their leadership heartbeat.
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(200);

/// Gets the local IP address by creating a UDP socket and checking which local address
/// would be used to reach an external destination. This doesn't actually send any traffic.
fn get_local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip())
}

/// Manages leadership election and heartbeat for DbElected nodes.
pub struct LeadershipElectionTask {
    backend: PostgresBackend,
    node_id: String,
    node_address: String,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
}

impl LeadershipElectionTask {
    /// Creates a new leadership election task.
    ///
    /// Connects to PostgreSQL without claiming leadership - the caller should
    /// call `try_acquire_leadership()` to attempt to become leader.
    pub async fn new(
        postgres_config: &PostgresConfig,
        shutdown_sender: watch::Sender<()>,
        bind_port: u16,
    ) -> Result<Self> {
        let backend = PostgresBackend::connect_without_leadership(postgres_config).await?;
        let shutdown_receiver = shutdown_sender.subscribe();

        // Compute node address from local IP and bind_port
        let local_ip = get_local_ip()
            .ok_or_else(|| anyhow::anyhow!("Failed to determine local IP address"))?;
        let node_address = format!("{}:{}", local_ip, bind_port);

        Ok(Self {
            backend,
            node_id: postgres_config.node_id.clone(),
            node_address,
            shutdown_sender,
            shutdown_receiver,
        })
    }

    /// Attempts to acquire leadership.
    ///
    /// Returns `true` if this node successfully became the leader.
    /// Returns `false` if another node is the leader.
    pub async fn try_acquire_leadership(&self) -> Result<bool> {
        match self.backend.try_update_leader().await? {
            Some(leader) => Ok(leader.node_id == self.node_id),
            None => Ok(false),
        }
    }

    /// Upserts this node's registration in the nodes table.
    async fn upsert_node_registration(&self) -> Result<()> {
        sqlx::query(
            "INSERT INTO nodes (node_id, address, last_updated)
             VALUES ($1, $2, NOW())
             ON CONFLICT (node_id) DO UPDATE
             SET address = EXCLUDED.address,
                 last_updated = NOW()",
        )
        .bind(&self.node_id)
        .bind(&self.node_address)
        .execute(self.backend.pool())
        .await?;
        Ok(())
    }

    /// Spawns the leader heartbeat task.
    ///
    /// This task periodically refreshes leadership and updates the node registration.
    /// If leadership is lost (another node took over or heartbeat failed), it triggers
    /// a graceful shutdown of the node.
    ///
    /// This method consumes `self` and should only be called after successfully
    /// acquiring leadership via `try_acquire_leadership()`.
    pub fn spawn_leader_heartbeat_task(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(node_id = %self.node_id, address = %self.node_address, "Starting leader heartbeat task");

            // Initial node registration
            if let Err(e) = self.upsert_node_registration().await {
                warn!(error = ?e, "Failed initial node registration");
            }

            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);

            loop {
                interval.tick().await;

                if self.shutdown_receiver.has_changed().unwrap_or(true) {
                    info!("Shutdown signal received, stopping heartbeat task");
                    return;
                }

                match self.try_acquire_leadership().await {
                    Ok(true) => {
                        // Successfully refreshed leadership
                        tracing::trace!("Leadership heartbeat successful");
                        // Also update node registration
                        if let Err(e) = self.upsert_node_registration().await {
                            warn!(error = ?e, "Failed to update node registration");
                        }
                    }
                    Ok(false) => {
                        error!(
                            node_id = %self.node_id,
                            "Leadership lost! Another node has taken over. Initiating graceful shutdown."
                        );
                        exit_rollup(&self.shutdown_sender).await;
                        unreachable!();
                    }
                    Err(e) => {
                        error!(
                            node_id = %self.node_id,
                            error = ?e,
                            "Heartbeat error! Unable to communicate with database. Initiating graceful shutdown."
                        );
                        exit_rollup(&self.shutdown_sender).await;
                        return;
                    }
                }
            }
        })
    }

    /// Spawns the replica election task.
    ///
    /// This task periodically attempts to acquire leadership and updates node registration.
    /// If leadership is acquired, it will transition the node from replica to leader.
    pub fn spawn_replica_election_task(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(node_id = %self.node_id, address = %self.node_address, "Starting replica election task");

            // Initial node registration
            if let Err(e) = self.upsert_node_registration().await {
                warn!(error = ?e, "Failed initial node registration");
            }

            let mut interval = tokio::time::interval(ELECTION_INTERVAL);

            loop {
                interval.tick().await;

                if self.shutdown_receiver.has_changed().unwrap_or(true) {
                    info!("Shutdown signal received, stopping election task");
                    return;
                }

                // Update node registration
                if let Err(e) = self.upsert_node_registration().await {
                    warn!(error = ?e, "Failed to update node registration");
                }

                match self.try_acquire_leadership().await {
                    Ok(true) => {
                        info!(
                            node_id = %self.node_id,
                            "Replica acquired leadership! Exiting to restart as leader."
                        );
                        let _ = self.shutdown_sender.send(());
                    }
                    Ok(false) => {
                        // Another node is still leader, keep trying
                        tracing::trace!("Leadership acquisition failed, another node is leader");
                    }
                    Err(e) => {
                        warn!(
                            node_id = %self.node_id,
                            error = ?e,
                            "Election attempt failed, will retry"
                        );
                    }
                }
            }
        })
    }
}
