use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

// `axum_ws_submit_tx` disconnects clients that fail to pong within 10s of a
// server ping. Recv intervals at or above this will get the connection killed.
const SERVER_PONG_TIMEOUT_SECS: u64 = 10;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Submit transactions to the sequencer's submit-ws endpoint at one cadence and drain replies at another, to reproduce per-connection task accumulation under reply backpressure."
)]
struct Args {
    /// WebSocket URL of the sequencer's submit endpoint.
    #[arg(long, default_value = "ws://127.0.0.1:12346/sequencer/txs/submit/ws")]
    url: String,

    /// How often to send a transaction (ms).
    #[arg(long, default_value_t = 100)]
    send_interval_ms: u64,

    /// How often to drain pending replies from the websocket (ms).
    /// Setting this near or above the server's 10s pong timeout will get the
    /// connection killed before backpressure can build up.
    #[arg(long, default_value_t = 1000)]
    recv_interval_ms: u64,

    /// How often to log running counters (ms).
    #[arg(long, default_value_t = 5000)]
    stats_interval_ms: u64,

    /// Run for at most this many seconds. If unset, run until Ctrl-C.
    #[arg(long)]
    duration_secs: Option<u64>,

    /// Number of bytes per generated tx body.
    #[arg(long, default_value_t = 128)]
    tx_body_bytes: usize,
}

#[derive(Serialize, Deserialize)]
struct WsMessage<T> {
    id: String,
    contents: T,
}

#[derive(Serialize, Deserialize)]
struct AcceptTx {
    body: String,
}

#[derive(Default)]
struct Counters {
    sent: u64,
    received: u64,
    pings: u64,
    errors: u64,
}

impl Counters {
    fn backlog(&self) -> u64 {
        self.sent.saturating_sub(self.received)
    }

    fn log(&self, label: &str) {
        tracing::info!(
            sent = self.sent,
            received = self.received,
            pings = self.pings,
            errors = self.errors,
            backlog = self.backlog(),
            "{label}",
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    if args.recv_interval_ms >= SERVER_PONG_TIMEOUT_SECS * 1000 {
        tracing::warn!(
            recv_interval_ms = args.recv_interval_ms,
            pong_timeout_secs = SERVER_PONG_TIMEOUT_SECS,
            "recv interval is at or above the server's pong timeout; expect the server to disconnect us before pile-up develops",
        );
    }

    tracing::info!(url = %args.url, "connecting");
    let (mut ws, _) = connect_async(&args.url)
        .await
        .with_context(|| format!("failed to connect to {}", args.url))?;
    tracing::info!(
        send_interval_ms = args.send_interval_ms,
        recv_interval_ms = args.recv_interval_ms,
        "connected",
    );

    let mut send_tick = interval(Duration::from_millis(args.send_interval_ms));
    let mut recv_tick = interval(Duration::from_millis(args.recv_interval_ms));
    let mut stats_tick = interval(Duration::from_millis(args.stats_interval_ms));
    let deadline = args
        .duration_secs
        .map(|s| Instant::now() + Duration::from_secs(s));

    let mut counters = Counters::default();

    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            tracing::info!("duration elapsed");
            break;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl-C received");
                break;
            }

            _ = send_tick.tick() => {
                let body = build_body(args.tx_body_bytes, counters.sent);
                let json = serde_json::to_string(&WsMessage {
                    id: format!("submit-{}", counters.sent),
                    contents: AcceptTx { body: BASE64.encode(&body) },
                })?;
                if let Err(e) = ws.send(Message::Text(json.into())).await {
                    tracing::error!(error = %e, "send failed");
                    counters.errors += 1;
                    break;
                }
                counters.sent += 1;
            }

            // Drain everything currently ready without blocking on the next frame.
            // The kernel TCP recv buffer fills between ticks, which is what creates
            // the server-side backpressure we are trying to reproduce.
            _ = recv_tick.tick() => {
                while let Ok(Some(maybe)) = tokio::time::timeout(Duration::ZERO, ws.next()).await {
                    match maybe {
                        Ok(Message::Text(_) | Message::Binary(_)) => counters.received += 1,
                        Ok(Message::Ping(p)) => {
                            counters.pings += 1;
                            if let Err(e) = ws.send(Message::Pong(p)).await {
                                tracing::error!(error = %e, "pong failed");
                                counters.errors += 1;
                                break;
                            }
                        }
                        Ok(Message::Pong(_) | Message::Frame(_)) => {}
                        Ok(Message::Close(frame)) => {
                            tracing::warn!(?frame, "server closed connection");
                            counters.log("done");
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "recv error");
                            counters.errors += 1;
                            break;
                        }
                    }
                }
            }

            _ = stats_tick.tick() => counters.log("stats"),
        }
    }

    let _ = ws.close(None).await;
    counters.log("done");
    Ok(())
}

/// Builds a `tx_body_bytes`-long buffer with the sequence number embedded in
/// the leading bytes so consecutive bodies are distinct on the wire.
///
/// The contents do not need to be a valid transaction: the sequencer spawns a
/// per-message task as soon as the JSON envelope parses (see
/// `crates/full-node/sov-sequencer/src/rest_api.rs:312-326`), and *that* task
/// is what blocks on the bounded reply channel when the client stops reading.
/// Validity only changes the rejection reason inside the task, not whether the
/// task gets spawned.
fn build_body(size: usize, seq: u64) -> Vec<u8> {
    let mut body = vec![0u8; size.max(8)];
    body[..8].copy_from_slice(&seq.to_le_bytes());
    body
}
