//! Heartbeat and leadership tasks for sequencer nodes.
//!
//! This module provides periodic background tasks that maintain node presence
//! in the cluster and handle leadership transitions:
//!
//! - **Leader nodes** run a heartbeat to maintain leadership; if lost, they shut down.
//! - **DbElected replicas** run a heartbeat while competing for leadership; if acquired, they restart as leader.
//! - **Static replicas** run a heartbeat for registration only, never competing for leadership.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::Result;
use sov_full_node_configs::sequencer::{ConfiguredNodeRole, PostgresConfig};
use sov_rollup_interface::node::{FutureOrShutdownOutput, PrimaryShutdownController};
use tokio::task::JoinHandle;
use tracing::{error, info, warn, Instrument};

use super::SequencerRole;

use super::postgres::PostgresBackend;
use crate::preferred::exit_rollup;

/// Leadership role of a node, used both as the configured mode of a heartbeat
/// task and as the result of a leadership heartbeat / election attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeadershipRole {
    /// This node currently holds leadership.
    Leader,
    /// This node is a replica: another node holds leadership, or no leader is
    /// currently elected.
    Replica,
}

impl std::fmt::Display for LeadershipRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeadershipRole::Leader => f.write_str("leader"),
            LeadershipRole::Replica => f.write_str("replica"),
        }
    }
}

/// Manages periodic heartbeat and optional leadership election for a sequencer node.
///
/// This task maintains the node's presence in the cluster by periodically updating
/// its registration in the database. Depending on the spawn method used, it may
/// also compete for leadership.
pub struct HeartBeatTask {
    backend: PostgresBackend,
    node_id: String,
    primary_shutdown: PrimaryShutdownController,
    postgres_config: PostgresConfig,
    heartbeat_interval: Duration,
}

impl HeartBeatTask {
    pub async fn new(
        postgres_config: PostgresConfig,
        primary_shutdown: PrimaryShutdownController,
        bind_addr: SocketAddr,
        heartbeat_interval: Duration,
    ) -> Result<Self> {
        let backend = PostgresBackend::connect(&postgres_config, bind_addr).await?;

        Ok(Self {
            backend,
            node_id: postgres_config.node_id.clone(),
            primary_shutdown,
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

    // Sends a heartbeat that competes for leadership and reports who holds it.
    async fn try_acquire_leadership(&self) -> Result<LeadershipRole> {
        match self
            .backend
            .heartbeat(Some(self.postgres_config.leader_election))
            .await?
        {
            Some(leader) if leader.node_id == self.node_id => Ok(LeadershipRole::Leader),
            _ => Ok(LeadershipRole::Replica),
        }
    }

    // Sends a heartbeat that only updates node registration (no leadership competition).
    async fn register_node(&self) -> Result<()> {
        self.backend.heartbeat(None).await?;
        Ok(())
    }

    // Best-effort deregistration of this node from the `nodes` table on shutdown.
    //
    // An SLO optimization, not a correctness requirement: on error or timeout we log and
    // continue, relying on staleness-based filtering as the fallback. Bounded so a slow or
    // unreachable database cannot delay shutdown.
    async fn deregister_on_shutdown_best_effort(&self) {
        const DEREGISTER_TIMEOUT: Duration = Duration::from_millis(500);
        match tokio::time::timeout(
            DEREGISTER_TIMEOUT,
            self.backend.deregister_node_on_shutdown(),
        )
        .await
        {
            Ok(Ok(())) => {
                info!(node_id = %self.node_id, "Deregistered node from cluster on shutdown");
            }
            Ok(Err(error)) => {
                warn!(node_id = %self.node_id, ?error, "Failed to deregister on shutdown; relying on staleness");
            }
            Err(_) => {
                warn!(node_id = %self.node_id, "Timed out deregistering on shutdown; relying on staleness");
            }
        }
    }

    // Spawns a task for the current leader to maintain leadership.
    //
    // Periodically refreshes leadership. If leadership is lost or the database
    // becomes unreachable, triggers a graceful shutdown.
    fn spawn_leader_heartbeat_task(self) -> JoinHandle<()> {
        self.spawn_heartbeat_loop(LeadershipRole::Leader)
    }

    // Spawns a task for a replica that wants to become leader.
    //
    // Periodically attempts to acquire leadership. If successful, triggers
    // a shutdown so the node can restart as the new leader.
    fn spawn_replica_heartbeat_task(self) -> JoinHandle<()> {
        self.spawn_heartbeat_loop(LeadershipRole::Replica)
    }

    // Drives the periodic heartbeat loop shared by the leader and replica tasks.
    //
    // Each tick acquires a fresh leadership status and dispatches it to the
    // mode-specific handler. The loop ends on shutdown, or when the handler
    // signals it should stop (e.g. a replica that won the election).
    fn spawn_heartbeat_loop(self, mode: LeadershipRole) -> JoinHandle<()> {
        // Attach the node identity, role, and interval to a span so every event
        // emitted by this task carries them, instead of repeating the fields on
        // each log.
        let span = tracing::info_span!(
            "heartbeat",
            node_id = %self.node_id,
            address = %self.backend.node_address,
            %mode,
            heartbeat_interval = ?self.heartbeat_interval,
        );

        tokio::spawn(
            async move {
                info!("Starting heartbeat task");
                let mut interval = tokio::time::interval(self.heartbeat_interval);

                // Liveness log, throttled to at most once every 5s. Helps detect a
                // blocked tokio event loop: a gap between these logs means the loop
                // was not being driven.
                let alive_log_period = Duration::from_secs(5);
                let mut last_alive_log = Instant::now();

                loop {
                    // Also helps detect a frozen tokio event loop: a stalled runtime
                    // makes the tick fire late, so `tick_elapsed` (below) overshoots
                    // the heartbeat interval.
                    let tick_start = Instant::now();
                    match self.primary_shutdown.future_or_shutdown(interval.tick()).await {
                        FutureOrShutdownOutput::Shutdown => {
                            info!("Shutdown signal received, stopping heartbeat task");
                            // Only a replica deregisters; the leader is removed via the
                            // staleness timeout, so deleting its `nodes` row here would be wrong.
                            if mode == LeadershipRole::Replica {
                                self.deregister_on_shutdown_best_effort().await;
                            }
                            return;
                        }
                        FutureOrShutdownOutput::Output(_) => {
                            let tick_elapsed = tick_start.elapsed();
                            if tick_elapsed > 2 * self.heartbeat_interval {
                                warn!(
                                    elapsed = ?tick_elapsed,
                                    "Heartbeat interval tick took longer than twice the heartbeat interval"
                                );
                            }

                            let start = Instant::now();
                            let status = self.try_acquire_leadership().await;
                            let elapsed = start.elapsed();
                            if elapsed > self.heartbeat_interval {
                                warn!(
                                    ?elapsed,
                                    "Leadership heartbeat db call took longer than the heartbeat interval"
                                );
                            }

                            if last_alive_log.elapsed() >= alive_log_period {
                                info!("Heartbeat task alive, sending heartbeats");
                                last_alive_log = Instant::now();
                            }

                            match mode {
                                LeadershipRole::Leader => self.handle_leader_heartbeat(status).await,
                                LeadershipRole::Replica => {
                                    if self.handle_replica_election(status) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .instrument(span),
        )
    }

    // Handles the outcome of a leader heartbeat. If leadership is lost or the
    // database is unreachable, triggers a graceful shutdown (never returns).
    async fn handle_leader_heartbeat(&self, status: Result<LeadershipRole>) {
        match status {
            Ok(LeadershipRole::Leader) => {
                // Successfully refreshed leadership and node registration
                tracing::trace!("Leadership heartbeat successful.");
            }
            Ok(LeadershipRole::Replica) => {
                error!(
                    node_id = %self.node_id,
                    "Leadership lost! Another node has taken over. Initiating graceful shutdown."
                );
                exit_rollup(&self.primary_shutdown).await;
            }
            Err(e) => {
                error!(
                    node_id = %self.node_id,
                    error = ?e,
                    "Heartbeat error! Unable to communicate with database. Initiating graceful shutdown."
                );
                exit_rollup(&self.primary_shutdown).await;
            }
        }
    }

    // Handles the outcome of a replica election attempt. Returns `true` if
    // leadership was acquired and the election loop should stop.
    fn handle_replica_election(&self, status: Result<LeadershipRole>) -> bool {
        match status {
            Ok(LeadershipRole::Leader) => {
                info!(
                    node_id = %self.node_id,
                    "Replica acquired leadership! Exiting to restart as leader."
                );
                self.primary_shutdown.shutdown();
                true
            }
            Ok(LeadershipRole::Replica) => {
                // Another node is still leader, keep trying
                tracing::trace!("Leadership acquisition failed, another node is leader.");
                false
            }
            Err(e) => {
                warn!(
                    node_id = %self.node_id,
                    error = ?e,
                    "Election attempt failed, will retry."
                );
                false
            }
        }
    }

    // Spawns a task that only maintains node registration.
    //
    // Periodically updates the node's entry in the `nodes` table without
    // competing for leadership. Failures are logged but don't cause shutdown.
    fn spawn_node_registration_task(self) -> JoinHandle<()> {
        let span = tracing::info_span!("registration", mode = %LeadershipRole::Replica);

        tokio::spawn(
            async move {
                info!(node_id = %self.node_id, address = %self.backend.node_address, "Starting replica registration task.");
                let mut interval = tokio::time::interval(self.heartbeat_interval);

                loop {
                    match self.primary_shutdown.future_or_shutdown(interval.tick()).await {
                        FutureOrShutdownOutput::Shutdown => {
                            info!(node_id = %self.node_id, "Shutdown signal received, stopping registration task.");
                            self.deregister_on_shutdown_best_effort().await;
                            return;
                        }
                        FutureOrShutdownOutput::Output(_) => match self.register_node().await {
                            Ok(_) => {}
                            Err(error) => {
                                warn!(
                                    node_id = %self.node_id,
                                    ?error,
                                    "Node registration attempt failed, will retry."
                                );
                            }
                        },
                    }
                }
            }
            .instrument(span),
        )
    }
}
