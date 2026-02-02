//! Heartbeat and leadership tasks for sequencer nodes.
//!
//! This module provides periodic background tasks that maintain node presence
//! in the cluster and handle leadership transitions:
//!
//! - **Leader nodes** run a heartbeat to maintain leadership; if lost, they shut down.
//! - **DbElected replicas** run a heartbeat while competing for leadership; if acquired, they restart as leader.
//! - **Static replicas** run a heartbeat for registration only, never competing for leadership.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use sov_full_node_configs::sequencer::{ConfiguredNodeRole, PostgresConfig};
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use super::SequencerRole;

use super::postgres::PostgresBackend;
use crate::preferred::exit_rollup;

/// Manages periodic heartbeat and optional leadership election for a sequencer node.                                                                                                                                                                        
///                                                                                                                                                                                                                                                          
/// This task maintains the node's presence in the cluster by periodically updating                                                                                                                                                                          
/// its registration in the database. Depending on the spawn method used, it may                                                                                                                                                                             
/// also compete for leadership.
pub struct HeartBeatTask {
    backend: PostgresBackend,
    node_id: String,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
    postgres_config: PostgresConfig,
    heartbeat_interval: Duration,
}

impl HeartBeatTask {
    pub async fn new(
        postgres_config: PostgresConfig,
        shutdown_sender: watch::Sender<()>,
        bind_addr: SocketAddr,
        heartbeat_interval: Duration,
    ) -> Result<Self> {
        let backend = PostgresBackend::connect(&postgres_config, bind_addr).await?;
        let shutdown_receiver = shutdown_sender.subscribe();

        Ok(Self {
            backend,
            node_id: postgres_config.node_id.clone(),
            shutdown_sender,
            shutdown_receiver,
            postgres_config,
            heartbeat_interval,
        })
    }

    /// Spawns heartbeat tasks based on the node's configured role.
    pub async fn spawn(self, seq_role: SequencerRole) -> JoinHandle<()> {
        let configures_node_role = self.postgres_config.node_role;
        match seq_role {
            SequencerRole::BatchProducer => self.spawn_leader_heartbeat_task(),
            SequencerRole::PgSyncReplica => {
                if configures_node_role == ConfiguredNodeRole::DbElected {
                    self.spawn_replica_heartbeat_task()
                } else {
                    assert_eq!(configures_node_role, ConfiguredNodeRole::Replica);
                    self.spawn_node_registration_task()
                }
            }
            SequencerRole::DaOnlyReplica => self.spawn_node_registration_task(),
        }
    }

    // Sends a heartbeat that competes for leadership.
    // Returns `true` if this node is the current leader.
    async fn try_acquire_leadership(&self) -> Result<bool> {
        match self
            .backend
            .heartbeat(Some(self.postgres_config.leader_election))
            .await?
        {
            Some(leader) => Ok(leader.node_id == self.node_id),
            None => Ok(false),
        }
    }

    // Sends a heartbeat that only updates node registration (no leadership competition).
    async fn register_node(&self) -> Result<()> {
        self.backend.heartbeat(None).await?;
        Ok(())
    }

    // Spawns a task for the current leader to maintain leadership.
    //
    // Periodically refreshes leadership. If leadership is lost or the database
    // becomes unreachable, triggers a graceful shutdown.  .
    fn spawn_leader_heartbeat_task(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting leader heartbeat task");
            let mut interval = tokio::time::interval(self.heartbeat_interval);

            loop {
                match future_or_shutdown(interval.tick(), &self.shutdown_receiver).await {
                    FutureOrShutdownOutput::Shutdown => {
                        info!("Shutdown signal received, stopping heartbeat task");
                        return;
                    }
                    FutureOrShutdownOutput::Output(_) => {
                        match self.try_acquire_leadership().await {
                            Ok(true) => {
                                // Successfully refreshed leadership and node registration
                                tracing::trace!("Leadership heartbeat successful.");
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
                                unreachable!();
                            }
                        }
                    }
                }
            }
        })
    }

    // Spawns a task for a replica that wants to become leader.
    //
    // Periodically attempts to acquire leadership. If successful, triggers
    // a shutdown so the node can restart as the new leader.
    fn spawn_replica_heartbeat_task(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting replica election task");
            let mut interval = tokio::time::interval(self.heartbeat_interval);

            loop {
                match future_or_shutdown(interval.tick(), &self.shutdown_receiver).await {
                    FutureOrShutdownOutput::Shutdown => {
                        info!("Shutdown signal received, stopping election task.");
                        return;
                    }
                    FutureOrShutdownOutput::Output(_) => {
                        match self.try_acquire_leadership().await {
                            Ok(true) => {
                                info!(
                                    node_id = %self.node_id,
                                    "Replica acquired leadership! Exiting to restart as leader."
                                );
                                let _ = self.shutdown_sender.send(());
                                break;
                            }
                            Ok(false) => {
                                // Another node is still leader, keep trying
                                tracing::trace!(
                                    "Leadership acquisition failed, another node is leader."
                                );
                            }
                            Err(e) => {
                                warn!(
                                    node_id = %self.node_id,
                                    error = ?e,
                                    "Election attempt failed, will retry."
                                );
                            }
                        }
                    }
                }
            }
        })
    }

    // Spawns a task that only maintains node registration.
    //
    // Periodically updates the node's entry in the `nodes` table without
    // competing for leadership. Failures are logged but don't cause shutdown.
    fn spawn_node_registration_task(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting replica registration task.");
            let mut interval = tokio::time::interval(self.heartbeat_interval);

            loop {
                match future_or_shutdown(interval.tick(), &self.shutdown_receiver).await {
                    FutureOrShutdownOutput::Shutdown => {
                        info!("Shutdown signal received, stopping registration task.");
                        return;
                    }
                    FutureOrShutdownOutput::Output(_) => match self.register_node().await {
                        Ok(_) => {}
                        Err(e) => {
                            warn!(
                                node_id = %self.node_id,
                                error = ?e,
                                "Node registration attempt failed, will retry."
                            );
                        }
                    },
                }
            }
        })
    }
}
