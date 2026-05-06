use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpSocket;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_with_config, connect_async, MaybeTlsStream};

// Keep this list simple on purpose so endpoints can be commented out by hand.
const DEFAULT_WS_PATHS: &[&str] = &[
    "/ledger/aggregated-proofs/latest/ws",
    "/ledger/slots/latest/ws",
    "/ledger/slots/latest/ws?children=1",
    "/ledger/slots/finalized/ws",
    "/ledger/slots/finalized/ws?children=1",
    "/ledger/slots/latest/events/ws",
];

// `serve_generic_ws_subscription()` starts pinging after 30s of inactivity, so
// the default pause stays below that threshold to avoid disconnecting before
// the read phase begins.
const SERVER_PING_INTERVAL_SECS: u64 = 30;

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Open several ledger websocket subscriptions, pause reads, then resume and log frame sizes."
)]
struct Args {
    /// Base HTTP or WS URL for the node.
    #[arg(long, default_value = "http://127.0.0.1:12346")]
    base_url: String,

    /// Relative websocket path to subscribe to. Repeat to override the built-in default list.
    #[arg(long = "path")]
    paths: Vec<String>,

    /// How long to keep each websocket open without reading from it.
    #[arg(long, default_value_t = 20)]
    pause_secs: u64,

    /// Maximum time to spend reading after the pause ends.
    #[arg(long, default_value_t = 20)]
    resume_secs: u64,

    /// Timeout for the initial TCP + WebSocket connection.
    #[arg(long, default_value_t = 10)]
    connect_timeout_secs: u64,

    /// Per-frame read timeout once the client resumes consuming.
    #[arg(long, default_value_t = 5)]
    read_timeout_secs: u64,

    /// Stop after this many data frames per endpoint.
    #[arg(long, default_value_t = 128)]
    max_frames: usize,

    /// Optional TCP SO_RCVBUF size to reduce kernel-side buffering.
    #[arg(long)]
    tcp_recv_buffer_bytes: Option<u32>,
}

#[derive(Debug)]
struct EndpointSummary {
    label: String,
    data_frames: usize,
    total_payload_bytes: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.pause_secs >= SERVER_PING_INTERVAL_SECS {
        eprintln!(
            "warning: --pause-secs={} is at or above the server ping interval of {}s; \
             expect the server to disconnect before resume unless you shorten the pause",
            args.pause_secs, SERVER_PING_INTERVAL_SECS
        );
    }

    let paths = if args.paths.is_empty() {
        DEFAULT_WS_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<_>>()
    } else {
        args.paths.clone()
    };

    let mut join_set = tokio::task::JoinSet::new();
    for path in paths {
        let args = args.clone();
        join_set.spawn(async move { run_endpoint(path, args).await });
    }

    let mut completed = 0usize;
    let mut failed = 0usize;

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok(Ok(summary)) => {
                completed += 1;
                println!(
                    "[{}] summary: data_frames={} total_payload_bytes={}",
                    summary.label, summary.data_frames, summary.total_payload_bytes
                );
            }
            Ok(Err(error)) => {
                failed += 1;
                eprintln!("endpoint task failed: {error:#}");
            }
            Err(error) => {
                failed += 1;
                eprintln!("endpoint task panicked or was cancelled: {error}");
            }
        }
    }

    ensure!(completed > 0, "all websocket subscriptions failed");

    if failed > 0 {
        eprintln!(
            "completed {} endpoint(s) successfully and {} failed",
            completed, failed
        );
    }

    Ok(())
}

async fn run_endpoint(path: String, args: Args) -> Result<EndpointSummary> {
    let label = label_for_path(&path);
    let ws_url = build_ws_url(&args.base_url, &path)?;

    println!("[{label}] connecting to {ws_url}");
    let connect_timeout = Duration::from_secs(args.connect_timeout_secs);
    let mut socket = tokio::time::timeout(
        connect_timeout,
        connect_ws(&ws_url, args.tcp_recv_buffer_bytes),
    )
    .await
    .with_context(|| format!("[{label}] connect timed out after {connect_timeout:?}"))??;

    println!(
        "[{label}] connected; pausing reads for {}s",
        args.pause_secs
    );
    tokio::time::sleep(Duration::from_secs(args.pause_secs)).await;

    println!(
        "[{label}] resuming reads for up to {}s or {} data frames",
        args.resume_secs, args.max_frames
    );

    let resume_deadline = Instant::now() + Duration::from_secs(args.resume_secs);
    let frame_timeout = Duration::from_secs(args.read_timeout_secs);
    let resume_started_at = Instant::now();
    let mut data_frames = 0usize;
    let mut total_payload_bytes = 0usize;

    while data_frames < args.max_frames {
        let remaining = resume_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            println!("[{label}] resume window elapsed");
            break;
        }

        let wait_for = remaining.min(frame_timeout);
        match tokio::time::timeout(wait_for, socket.next()).await {
            Err(_) => {
                println!(
                    "[{label}] no websocket frame received within {:?}; stopping",
                    wait_for
                );
                break;
            }
            Ok(None) => {
                println!("[{label}] server closed the websocket");
                break;
            }
            Ok(Some(Err(error))) => {
                println!("[{label}] websocket read error: {error}");
                break;
            }
            Ok(Some(Ok(message))) => {
                if handle_message(
                    &label,
                    &mut socket,
                    message,
                    resume_started_at,
                    &mut data_frames,
                    &mut total_payload_bytes,
                )
                .await?
                {
                    break;
                }
            }
        }
    }

    socket.close(None).await.ok();

    Ok(EndpointSummary {
        label,
        data_frames,
        total_payload_bytes,
    })
}

async fn handle_message<S>(
    label: &str,
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: Message,
    resume_started_at: Instant,
    data_frames: &mut usize,
    total_payload_bytes: &mut usize,
) -> Result<bool>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let elapsed_ms = resume_started_at.elapsed().as_millis();
    match message {
        Message::Text(text) => {
            *data_frames += 1;
            *total_payload_bytes += text.len();
            println!(
                "[{label}] +{elapsed_ms}ms frame#{} text {} bytes total={}",
                *data_frames,
                text.len(),
                *total_payload_bytes
            );
            Ok(false)
        }
        Message::Binary(bytes) => {
            *data_frames += 1;
            *total_payload_bytes += bytes.len();
            println!(
                "[{label}] +{elapsed_ms}ms frame#{} binary {} bytes total={}",
                *data_frames,
                bytes.len(),
                *total_payload_bytes
            );
            Ok(false)
        }
        Message::Ping(bytes) => {
            println!("[{label}] +{elapsed_ms}ms ping {} bytes", bytes.len());
            socket
                .send(Message::Pong(bytes))
                .await
                .map_err(|error| anyhow!("[{label}] failed to reply with pong: {error}"))?;
            Ok(false)
        }
        Message::Pong(bytes) => {
            println!("[{label}] +{elapsed_ms}ms pong {} bytes", bytes.len());
            Ok(false)
        }
        Message::Close(frame) => {
            println!("[{label}] +{elapsed_ms}ms close frame: {frame:?}");
            Ok(true)
        }
        other => {
            println!("[{label}] +{elapsed_ms}ms ignored frame: {other:?}");
            Ok(false)
        }
    }
}

async fn connect_ws(
    ws_url: &str,
    tcp_recv_buffer_bytes: Option<u32>,
) -> Result<tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
    let request = ws_url.into_client_request()?;
    let uri = request.uri();
    let scheme = uri
        .scheme_str()
        .context("websocket URL is missing a scheme")?;

    if scheme == "wss" {
        ensure!(
            tcp_recv_buffer_bytes.is_none(),
            "--tcp-recv-buffer-bytes is only supported for ws:// and http:// URLs"
        );
        let (ws, _) = connect_async(ws_url)
            .await
            .context("websocket TLS handshake failed")?;
        return Ok(ws);
    }

    ensure!(scheme == "ws", "unsupported websocket URL scheme: {scheme}");

    let host = uri.host().context("websocket URL is missing a host")?;
    let port = uri.port_u16().unwrap_or(80);
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow!("no socket addresses resolved for {host}:{port}"))?;

    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }?;

    if let Some(size) = tcp_recv_buffer_bytes {
        socket
            .set_recv_buffer_size(size)
            .with_context(|| format!("failed to set SO_RCVBUF={size}"))?;
    }

    let stream = socket
        .connect(addr)
        .await
        .with_context(|| format!("failed to connect to {addr}"))?;
    let stream = MaybeTlsStream::Plain(stream);
    let (ws, _) = client_async_with_config(request, stream, None)
        .await
        .context("websocket handshake failed")?;
    Ok(ws)
}

fn build_ws_url(base_url: &str, path: &str) -> Result<String> {
    let normalized_base = base_url.trim_end_matches('/');
    let normalized_path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let url = format!("{normalized_base}{normalized_path}");

    if let Some(url) = url.strip_prefix("http://") {
        return Ok(format!("ws://{url}"));
    }
    if let Some(url) = url.strip_prefix("ws://") {
        return Ok(format!("ws://{url}"));
    }
    if let Some(url) = url.strip_prefix("https://") {
        return Ok(format!("wss://{url}"));
    }
    if let Some(url) = url.strip_prefix("wss://") {
        return Ok(format!("wss://{url}"));
    }

    bail!("base URL must start with http://, https://, ws://, or wss://: {base_url}");
}

fn label_for_path(path: &str) -> String {
    path.trim_start_matches('/')
        .replace(['/', '?', '=', '&'], "_")
}
