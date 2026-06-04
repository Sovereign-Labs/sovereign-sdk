//! Integration tests for the follower readiness probe that gates the
//! advertised cluster view.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use sov_proxy_utils::{ClusterInfo, ClusterUpdateNotifier};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

mod common;
use common::{insert_node, set_leader, setup, start_discovery, wait_for_change};

const READY_NODE: &str = "ready_node";
const LATE_READY_NODE: &str = "late_ready_node";
const LEADER_NODE: &str = "leader_node";

/// Records every advertised cluster update so a test can assert on the
/// sequence of advertised views.
///
/// The companion [`Sender`] implements [`ClusterUpdateNotifier`] and pushes
/// updates into an mpsc channel; this struct owns the receiver and exposes
/// helpers for draining it.
struct ClusterUpdateRecorder {
    rx: UnboundedReceiver<ClusterInfo>,
}

impl ClusterUpdateRecorder {
    fn new() -> (Box<dyn ClusterUpdateNotifier>, Self) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Box::new(Sender { tx }), Self { rx })
    }

    async fn next(&mut self) -> ClusterInfo {
        tokio::time::timeout(Duration::from_secs(10), self.rx.recv())
            .await
            .expect("Timed out waiting for an advertised cluster update")
            .expect("Notifier channel closed")
    }

    async fn wait_until_has_follower(&mut self, node_id: &str) -> ClusterInfo {
        loop {
            let info = self.next().await;
            if info.has_follower(node_id) {
                return info;
            }
        }
    }
}

struct Sender {
    tx: UnboundedSender<ClusterInfo>,
}

#[async_trait]
impl ClusterUpdateNotifier for Sender {
    async fn on_cluster_update(&mut self, cluster_info: &ClusterInfo) -> anyhow::Result<()> {
        let _ = self.tx.send(cluster_info.clone());
        Ok(())
    }
}

/// A tiny axum server whose `/sequencer/ready` route returns the currently configured status code.
struct ReadyEndpoint {
    address: SocketAddr,
    handle: JoinHandle<()>,
    status: Arc<AtomicU16>,
}

impl ReadyEndpoint {
    async fn start(status: StatusCode) -> Self {
        let status = Arc::new(AtomicU16::new(status.as_u16()));
        let status_for_handler = status.clone();
        let app = Router::new().route(
            "/sequencer/ready",
            get(move || {
                let status_for_handler = status_for_handler.clone();
                async move {
                    StatusCode::from_u16(status_for_handler.load(Ordering::SeqCst))
                        .expect("Status stored in AtomicU16 must be a valid HTTP status code")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind ready endpoint");
        let address = listener.local_addr().expect("local_addr failed");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            handle,
            status,
        }
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    fn set_status(&self, status: StatusCode) {
        self.status.store(status.as_u16(), Ordering::SeqCst);
    }
}

impl Drop for ReadyEndpoint {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// A follower whose `/sequencer/ready` endpoint returns a non-success status
/// is filtered out of the advertised cluster, while still appearing in the
/// raw watch channel.
#[tokio::test(flavor = "multi_thread")]
async fn not_ready_follower_is_excluded_from_advertised_cluster() {
    let Some((_container, connection_string, writer)) = setup().await else {
        return; // Docker unavailable — skip.
    };

    let ready_endpoint = ReadyEndpoint::start(StatusCode::OK).await;
    let late_ready_endpoint = ReadyEndpoint::start(StatusCode::SERVICE_UNAVAILABLE).await;

    let (notifier, mut recorder) = ClusterUpdateRecorder::new();

    // Short poll interval keeps the test fast; the readiness probe runs as
    // part of each poll cycle.
    let mut task = start_discovery(
        &connection_string,
        Duration::from_millis(500),
        Some(notifier),
    )
    .await;

    insert_node(&writer, READY_NODE, &ready_endpoint.address().to_string()).await;
    insert_node(
        &writer,
        LATE_READY_NODE,
        &late_ready_endpoint.address().to_string(),
    )
    .await;

    // Wait for the raw watch channel to publish a snapshot that includes both
    // followers, confirming both have been registered in the DB before we
    // assert on how the readiness probe filters the advertised view.
    loop {
        wait_for_change(&mut task).await;
        let info = task.receiver.borrow_and_update().clone();
        if info.has_follower(READY_NODE) && info.has_follower(LATE_READY_NODE) {
            break;
        }
    }

    // Drain advertised updates until the ready follower has been observed.
    // The discovery task may emit an intermediate snapshot before both nodes
    // are visible; in steady state the not-ready node is always filtered.
    let advertised = recorder.wait_until_has_follower(READY_NODE).await;

    assert!(
        advertised.has_follower(READY_NODE),
        "ready follower should be advertised"
    );
    assert!(
        !advertised.has_follower(LATE_READY_NODE),
        "follower returning 503 from /sequencer/ready must be excluded from the advertised cluster"
    );

    late_ready_endpoint.set_status(StatusCode::OK);
    let advertised = recorder.wait_until_has_follower(LATE_READY_NODE).await;

    assert!(
        advertised.has_follower(READY_NODE) && advertised.has_follower(LATE_READY_NODE),
        "after the previously not-ready follower starts returning 200, both followers should be advertised"
    );

    task.abort();
}

/// The leader is never readiness-probed: a leader whose `/sequencer/ready`
/// endpoint reports not-ready is still advertised as the cluster leader,
/// unlike followers which are gated on the probe.
#[tokio::test(flavor = "multi_thread")]
async fn not_ready_leader_is_still_advertised() {
    let Some((_container, connection_string, writer)) = setup().await else {
        return; // Docker unavailable — skip.
    };

    // The leader reports not-ready for the entire test; the follower is the
    // ready positive control that proves a readiness probe cycle ran.
    let leader_endpoint = ReadyEndpoint::start(StatusCode::SERVICE_UNAVAILABLE).await;
    let follower_endpoint = ReadyEndpoint::start(StatusCode::OK).await;

    let (notifier, mut recorder) = ClusterUpdateRecorder::new();

    let task = start_discovery(
        &connection_string,
        Duration::from_millis(500),
        Some(notifier),
    )
    .await;

    // Register and elect the leader *before* inserting the follower so that any
    // poll which observes the follower also observes the already-committed
    // leader row: the snapshot we assert on is guaranteed to carry the leader.
    insert_node(&writer, LEADER_NODE, &leader_endpoint.address().to_string()).await;
    set_leader(&writer, LEADER_NODE).await;
    insert_node(&writer, READY_NODE, &follower_endpoint.address().to_string()).await;

    // Wait for the ready follower to appear, confirming the probe cycle ran.
    let advertised = recorder.wait_until_has_follower(READY_NODE).await;

    assert!(
        advertised.has_leader(LEADER_NODE),
        "leader returning 503 from /sequencer/ready must still be advertised as leader"
    );

    task.abort();
}
