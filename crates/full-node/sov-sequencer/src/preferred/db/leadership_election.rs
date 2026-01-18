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

/// Computes the node address from the local IP and bind port.
pub fn compute_node_address(bind_port: u16) -> Result<String> {
    let local_ip =
        get_local_ip().ok_or_else(|| anyhow::anyhow!("Failed to determine local IP address"))?;
    Ok(format!("{local_ip}:{bind_port}"))
}

/// Manages leadership election and heartbeat for DbElected nodes.
pub struct LeadershipElectionTask {
    backend: PostgresBackend,
    node_id: String,
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
        let backend = PostgresBackend::connect(postgres_config, bind_port).await?;
        let shutdown_receiver = shutdown_sender.subscribe();

        Ok(Self {
            backend,
            node_id: postgres_config.node_id.clone(),
            shutdown_sender,
            shutdown_receiver,
        })
    }

    /// Attempts to acquire leadership and registers the node in the nodes table.
    ///
    /// Returns `true` if this node successfully became the leader.
    /// Returns `false` if another node is the leader.
    ///
    /// This method always registers the node in the nodes table, regardless of
    /// whether leadership was acquired.
    pub async fn try_acquire_leadership(&self) -> Result<bool> {
        match self.backend.try_update_leader_and_register_node().await? {
            Some(leader) => Ok(leader.node_id == self.node_id),
            None => Ok(false),
        }
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
            info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting leader heartbeat task");

            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);

            loop {
                interval.tick().await;

                if self.shutdown_receiver.has_changed().unwrap_or(true) {
                    info!("Shutdown signal received, stopping heartbeat task");
                    return;
                }

                match self.try_acquire_leadership().await {
                    Ok(true) => {
                        // Successfully refreshed leadership and node registration
                        tracing::trace!("Leadership heartbeat successful");
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
            info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting replica election task");

            let mut interval = tokio::time::interval(ELECTION_INTERVAL);

            loop {
                interval.tick().await;

                if self.shutdown_receiver.has_changed().unwrap_or(true) {
                    info!("Shutdown signal received, stopping election task");
                    return;
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
