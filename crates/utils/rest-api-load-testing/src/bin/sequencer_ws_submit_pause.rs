use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use borsh::to_vec;
use clap::Parser;
use demo_stf::runtime::{Runtime, RuntimeCall, CHAIN_HASH};
use demo_stf::MultiAddressEvmSolana;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sov_bank::{config_gas_token_id, Coins, TokenId};
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, PrivateKey as _, Spec};
use sov_node_client::NodeClient;
use sov_rollup_interface::da::DaSpec;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_test_utils::default_test_signed_transaction_with_nonce;
use tokio::net::TcpSocket;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{interval, timeout, Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_with_config, connect_async, MaybeTlsStream, WebSocketStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

type DemoStorage = NomtProverStorage<
    DefaultStorageSpec<<MockZkvmCryptoSpec as CryptoSpec>::Hasher>,
    <MockDaSpec as DaSpec>::SlotHash,
>;
type DemoRollupSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
    MockZkvmCryptoSpec,
    DemoStorage,
>;

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Cycle submit-ws clients through pause/drain phases to reproduce sequencer reply backpressure with real demo-rollup bank transfers."
)]
struct Args {
    /// WebSocket URL of the sequencer submit endpoint.
    #[arg(long, default_value = "ws://127.0.0.1:12346/sequencer/txs/submit/ws")]
    url: String,

    /// Number of independent websocket clients to run against the same submit-ws URL.
    #[arg(long, default_value_t = 1)]
    instances: usize,

    /// How often to send a transaction (ms).
    #[arg(long, default_value_t = 100)]
    send_interval_ms: u64,

    /// How long to pause reads while continuing to send (ms).
    #[arg(long, default_value_t = 20_000)]
    pause_recv_ms: u64,

    /// How long to drain reads while continuing to send (ms).
    #[arg(long, default_value_t = 2_000)]
    drain_recv_ms: u64,

    /// How often to log running counters (ms).
    #[arg(long, default_value_t = 5_000)]
    stats_interval_ms: u64,

    /// Number of connect/pause/drain/close cycles to run per instance.
    /// If omitted, each instance loops forever until interrupted.
    #[arg(long)]
    cycles: Option<usize>,

    /// Delay between cycles to avoid tight reconnect storms (ms).
    #[arg(long, default_value_t = 250)]
    cycle_delay_ms: u64,

    /// Optional TCP SO_RCVBUF size. Supported only for ws:// URLs.
    #[arg(long)]
    tcp_recv_buffer_bytes: Option<u32>,

    /// Private key file for the funded bank-transfer signer.
    #[arg(long, default_value_os_t = default_signer_key_file())]
    signer_key_file: PathBuf,

    /// Optional private key file whose address receives the bank transfers.
    #[arg(long)]
    recipient_key_file: Option<PathBuf>,

    /// Transfer amount used for each submitted bank transfer.
    #[arg(long, default_value_t = 1)]
    transfer_amount: u128,

    /// Token ID to transfer. Defaults to the configured gas token when omitted.
    #[arg(long)]
    token_id: Option<TokenId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WsTransport {
    Plain,
    Tls,
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

#[derive(Deserialize)]
struct SubmitWsResponseEnvelope {
    contents: Value,
}

#[derive(Clone)]
struct SubmitTxConfig {
    signer: PrivateKeyAndAddress<DemoRollupSpec>,
    recipient: <DemoRollupSpec as Spec>::Address,
    token_id: TokenId,
    transfer_amount: u128,
    nonce_stride: u64,
}

struct HarnessSetup {
    submit_tx: SubmitTxConfig,
    starting_nonce: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubmitWsInstance {
    instance_index: usize,
    starting_nonce: u64,
}

impl SubmitWsInstance {
    fn label(&self) -> String {
        format!("submit_ws#{:02}", self.instance_index)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    sent: u64,
    received: u64,
    pings: u64,
    errors: u64,
    max_backlog: u64,
}

impl Counters {
    fn backlog(&self) -> u64 {
        self.sent.saturating_sub(self.received)
    }

    fn record_send(&mut self) {
        self.sent += 1;
        self.max_backlog = self.max_backlog.max(self.backlog());
    }

    fn record_receive(&mut self) {
        self.received += 1;
        self.max_backlog = self.max_backlog.max(self.backlog());
    }

    fn record_ping(&mut self) {
        self.pings += 1;
        self.max_backlog = self.max_backlog.max(self.backlog());
    }

    fn record_error(&mut self) {
        self.errors += 1;
        self.max_backlog = self.max_backlog.max(self.backlog());
    }

    fn merge(&mut self, other: &Self) {
        self.sent += other.sent;
        self.received += other.received;
        self.pings += other.pings;
        self.errors += other.errors;
        self.max_backlog = self.max_backlog.max(other.max_backlog);
    }
}

#[derive(Debug)]
struct InstanceSummary {
    label: String,
    cycles_completed: usize,
    counters: Counters,
}

#[derive(Debug)]
struct InstanceReport {
    summary: InstanceSummary,
    error: Option<anyhow::Error>,
}

#[derive(Debug, Default)]
struct AggregateSummary {
    instances_completed: usize,
    instances_failed: usize,
    cycles_completed: usize,
    counters: Counters,
}

impl AggregateSummary {
    fn record(&mut self, report: &InstanceReport) {
        if report.error.is_some() {
            self.instances_failed += 1;
        } else {
            self.instances_completed += 1;
        }

        self.cycles_completed += report.summary.cycles_completed;
        self.counters.merge(&report.summary.counters);
    }
}

enum CycleExit {
    Completed,
    Shutdown,
}

enum PhaseExit {
    Continue,
    Shutdown,
}

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let transport = validate_args(&args)?;
    let harness_setup = prepare_harness(&args).await?;

    if args.tcp_recv_buffer_bytes.is_none() {
        tracing::warn!(
            "no --tcp-recv-buffer-bytes was provided; submit-ws backpressure reproduction is less reliable without shrinking the client receive buffer"
        );
    }
    if matches!(transport, WsTransport::Tls) {
        tracing::warn!(
            "wss:// connections cannot use --tcp-recv-buffer-bytes; submit-ws backpressure reproduction is less reliable than ws://"
        );
    }

    tracing::info!(
        url = %args.url,
        instances = args.instances,
        send_interval_ms = args.send_interval_ms,
        pause_recv_ms = args.pause_recv_ms,
        drain_recv_ms = args.drain_recv_ms,
        cycles = ?args.cycles,
        cycle_delay_ms = args.cycle_delay_ms,
        tcp_recv_buffer_bytes = ?args.tcp_recv_buffer_bytes,
        signer_key_file = %args.signer_key_file.display(),
        recipient_key_file = ?args.recipient_key_file.as_ref().map(|path| path.display().to_string()),
        signer_address = %harness_setup.submit_tx.signer.address,
        recipient_address = %harness_setup.submit_tx.recipient,
        transfer_amount = harness_setup.submit_tx.transfer_amount,
        token_id = %harness_setup.submit_tx.token_id,
        starting_nonce = harness_setup.starting_nonce,
        "starting submit-ws pressure harness",
    );

    let instances = build_instances(args.instances, harness_setup.starting_nonce)?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut join_set = JoinSet::new();

    for instance in instances {
        let args = args.clone();
        let shutdown = shutdown_rx.clone();
        let submit_tx = harness_setup.submit_tx.clone();
        join_set.spawn(async move { run_instance(instance, submit_tx, args, shutdown).await });
    }

    let mut join_failures = 0usize;
    let mut reports = Vec::with_capacity(args.instances);
    let mut shutdown_requested = false;

    while !join_set.is_empty() {
        tokio::select! {
            _ = tokio::signal::ctrl_c(), if !shutdown_requested => {
                shutdown_requested = true;
                tracing::warn!("Ctrl-C received; shutting down submit-ws instances after the current phase");
                let _ = shutdown_tx.send(true);
            }
            maybe_joined = join_set.join_next() => {
                if let Some(joined) = maybe_joined {
                    match joined {
                        Ok(report) => reports.push(report),
                        Err(error) => {
                            join_failures += 1;
                            tracing::error!(error = %error, "submit-ws instance task panicked or was cancelled");
                        }
                    }
                }
            }
        }
    }

    let mut aggregate = AggregateSummary::default();
    for report in &reports {
        log_instance_summary(&report.summary);
        if let Some(error) = &report.error {
            tracing::error!(
                label = %report.summary.label,
                error = %error,
                "submit-ws instance failed"
            );
        }
        aggregate.record(report);
    }

    let total_failed = aggregate.instances_failed + join_failures;
    tracing::info!(
        instances_total = args.instances,
        instances_completed = aggregate.instances_completed,
        instances_failed = total_failed,
        cycles_completed = aggregate.cycles_completed,
        sent = aggregate.counters.sent,
        received = aggregate.counters.received,
        pings = aggregate.counters.pings,
        errors = aggregate.counters.errors,
        backlog = aggregate.counters.backlog(),
        max_backlog = aggregate.counters.max_backlog,
        "submit-ws overall summary",
    );

    ensure!(
        total_failed == 0,
        "{total_failed} submit-ws instance(s) failed"
    );
    Ok(())
}

async fn run_instance(
    instance: SubmitWsInstance,
    submit_tx: SubmitTxConfig,
    args: Args,
    mut shutdown: watch::Receiver<bool>,
) -> InstanceReport {
    let label = instance.label();
    let mut next_nonce = instance.starting_nonce;
    let mut summary = InstanceSummary {
        label: label.clone(),
        cycles_completed: 0,
        counters: Counters::default(),
    };
    let mut cycle_number = 1usize;
    let mut error = None;

    loop {
        if *shutdown.borrow() {
            tracing::info!(
                label = %label,
                "shutdown requested before starting the next cycle"
            );
            break;
        }

        match run_cycle(
            &label,
            cycle_number,
            &args,
            &submit_tx,
            &mut next_nonce,
            &mut summary.counters,
            &mut shutdown,
        )
        .await
        {
            Ok(CycleExit::Completed) => {
                summary.cycles_completed += 1;
                if args
                    .cycles
                    .is_some_and(|target| summary.cycles_completed >= target)
                {
                    break;
                }
            }
            Ok(CycleExit::Shutdown) => break,
            Err(cycle_error) => {
                if args.cycles.is_some() {
                    error =
                        Some(cycle_error.context(format!("[{label}] cycle#{cycle_number} failed")));
                    break;
                }

                tracing::error!(
                    label = %label,
                    cycle_number,
                    error = %cycle_error,
                    "cycle failed; retrying after cycle delay",
                );
            }
        }

        cycle_number += 1;
        if matches!(
            wait_for_cycle_delay(&label, cycle_number, args.cycle_delay_ms, &mut shutdown).await,
            PhaseExit::Shutdown
        ) {
            break;
        }
    }

    InstanceReport { summary, error }
}

async fn run_cycle(
    label: &str,
    cycle_number: usize,
    args: &Args,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<CycleExit> {
    tracing::info!(
        label = %label,
        cycle_number,
        url = %args.url,
        next_nonce = *next_nonce,
        "cycle connect phase"
    );
    let mut ws = match timeout(
        CONNECT_TIMEOUT,
        connect_ws(&args.url, args.tcp_recv_buffer_bytes),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(error)) => {
            counters.record_error();
            return Err(error).with_context(|| {
                format!(
                    "[{label}] cycle#{cycle_number} failed to connect to {}",
                    args.url
                )
            });
        }
        Err(_) => {
            counters.record_error();
            return Err(anyhow!(
                "[{label}] cycle#{cycle_number} connect timed out after {:?}",
                CONNECT_TIMEOUT
            ));
        }
    };

    let mut send_tick = interval(Duration::from_millis(args.send_interval_ms));
    send_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stats_tick = interval(Duration::from_millis(args.stats_interval_ms));
    stats_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    tracing::info!(
        label = %label,
        cycle_number,
        pause_recv_ms = args.pause_recv_ms,
        "cycle pause phase: reads paused while sends continue",
    );
    let pause_result = run_pause_phase(
        label,
        cycle_number,
        Duration::from_millis(args.pause_recv_ms),
        &mut ws,
        &mut send_tick,
        &mut stats_tick,
        submit_tx,
        next_nonce,
        counters,
        shutdown,
    )
    .await;
    match pause_result {
        Ok(PhaseExit::Continue) => {}
        Ok(PhaseExit::Shutdown) => {
            close_ws_best_effort(&mut ws, label, cycle_number).await;
            return Ok(CycleExit::Shutdown);
        }
        Err(error) => {
            close_ws_best_effort(&mut ws, label, cycle_number).await;
            return Err(error);
        }
    }

    tracing::info!(
        label = %label,
        cycle_number,
        drain_recv_ms = args.drain_recv_ms,
        "cycle drain phase: draining replies while sends continue",
    );
    let drain_result = run_drain_phase(
        label,
        cycle_number,
        Duration::from_millis(args.drain_recv_ms),
        &mut ws,
        &mut send_tick,
        &mut stats_tick,
        submit_tx,
        next_nonce,
        counters,
        shutdown,
    )
    .await;
    match drain_result {
        Ok(PhaseExit::Continue) => {}
        Ok(PhaseExit::Shutdown) => {
            close_ws_best_effort(&mut ws, label, cycle_number).await;
            return Ok(CycleExit::Shutdown);
        }
        Err(error) => {
            close_ws_best_effort(&mut ws, label, cycle_number).await;
            return Err(error);
        }
    }

    tracing::info!(label = %label, cycle_number, "cycle close phase");
    if let Err(error) = ws.close(None).await {
        counters.record_error();
        return Err(anyhow!(
            "[{label}] cycle#{cycle_number} failed to close websocket: {error}"
        ));
    }

    tracing::info!(
        label = %label,
        cycle_number,
        next_nonce = *next_nonce,
        sent = counters.sent,
        received = counters.received,
        pings = counters.pings,
        errors = counters.errors,
        backlog = counters.backlog(),
        max_backlog = counters.max_backlog,
        "cycle completed",
    );
    Ok(CycleExit::Completed)
}

async fn run_pause_phase(
    label: &str,
    cycle_number: usize,
    duration: Duration,
    ws: &mut WsStream,
    send_tick: &mut Interval,
    stats_tick: &mut Interval,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<PhaseExit> {
    let deadline = Instant::now() + duration;
    let deadline_sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(deadline_sleep);

    loop {
        tokio::select! {
            biased;

            changed = shutdown.changed() => {
                if shutdown_triggered(changed, shutdown) {
                    return Ok(PhaseExit::Shutdown);
                }
            }
            _ = &mut deadline_sleep => return Ok(PhaseExit::Continue),
            _ = send_tick.tick() => {
                send_submit_tx(label, ws, submit_tx, next_nonce, counters).await
                    .with_context(|| format!("[{label}] cycle#{cycle_number} pause send failed"))?;
            }
            _ = stats_tick.tick() => {
                log_phase_stats(label, cycle_number, "pause", counters);
            }
        }
    }
}

async fn run_drain_phase(
    label: &str,
    cycle_number: usize,
    duration: Duration,
    ws: &mut WsStream,
    send_tick: &mut Interval,
    stats_tick: &mut Interval,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<PhaseExit> {
    let deadline = Instant::now() + duration;
    let deadline_sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(deadline_sleep);

    loop {
        tokio::select! {
            biased;

            changed = shutdown.changed() => {
                if shutdown_triggered(changed, shutdown) {
                    return Ok(PhaseExit::Shutdown);
                }
            }
            _ = &mut deadline_sleep => return Ok(PhaseExit::Continue),
            _ = send_tick.tick() => {
                send_submit_tx(label, ws, submit_tx, next_nonce, counters).await
                    .with_context(|| format!("[{label}] cycle#{cycle_number} drain send failed"))?;
            }
            _ = stats_tick.tick() => {
                log_phase_stats(label, cycle_number, "drain", counters);
            }
            next_message = ws.next() => {
                handle_inbound_message(label, cycle_number, ws, next_message, counters).await?;
                match drain_ready_messages(
                    label,
                    cycle_number,
                    ws,
                    send_tick,
                    stats_tick,
                    submit_tx,
                    next_nonce,
                    counters,
                    shutdown,
                )
                .await? {
                    PhaseExit::Continue => {}
                    PhaseExit::Shutdown => return Ok(PhaseExit::Shutdown),
                }
            }
        }
    }
}

async fn drain_ready_messages(
    label: &str,
    cycle_number: usize,
    ws: &mut WsStream,
    send_tick: &mut Interval,
    stats_tick: &mut Interval,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
    shutdown: &watch::Receiver<bool>,
) -> Result<PhaseExit> {
    loop {
        if *shutdown.borrow() {
            return Ok(PhaseExit::Shutdown);
        }

        if service_ready_drain_timers(
            label,
            cycle_number,
            ws,
            send_tick,
            stats_tick,
            submit_tx,
            next_nonce,
            counters,
        )
        .await?
        {
            continue;
        }

        match timeout(Duration::ZERO, ws.next()).await {
            Ok(next_message) => {
                handle_inbound_message(label, cycle_number, ws, next_message, counters).await?;
            }
            Err(_) => return Ok(PhaseExit::Continue),
        }
    }
}

async fn service_ready_drain_timers(
    label: &str,
    cycle_number: usize,
    ws: &mut WsStream,
    send_tick: &mut Interval,
    stats_tick: &mut Interval,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
) -> Result<bool> {
    if timeout(Duration::ZERO, send_tick.tick()).await.is_ok() {
        send_submit_tx(label, ws, submit_tx, next_nonce, counters)
            .await
            .with_context(|| format!("[{label}] cycle#{cycle_number} drain send failed"))?;
        return Ok(true);
    }

    if timeout(Duration::ZERO, stats_tick.tick()).await.is_ok() {
        log_phase_stats(label, cycle_number, "drain", counters);
        return Ok(true);
    }

    Ok(false)
}

async fn handle_inbound_message(
    label: &str,
    cycle_number: usize,
    ws: &mut WsStream,
    next_message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    counters: &mut Counters,
) -> Result<()> {
    match next_message {
        Some(Ok(Message::Text(text))) => {
            counters.record_receive();
            if submit_ws_text_is_error(text.as_ref()) {
                counters.record_error();
                tracing::warn!(
                    label = %label,
                    cycle_number,
                    response = %text,
                    "submit-ws transaction was rejected by the node",
                );
            }
            Ok(())
        }
        Some(Ok(Message::Binary(_))) => {
            counters.record_receive();
            Ok(())
        }
        Some(Ok(Message::Ping(payload))) => {
            counters.record_ping();
            ws.send(Message::Pong(payload)).await.map_err(|error| {
                counters.record_error();
                anyhow!("[{label}] cycle#{cycle_number} failed to reply to ping: {error}")
            })?;
            Ok(())
        }
        Some(Ok(Message::Pong(_) | Message::Frame(_))) => Ok(()),
        Some(Ok(Message::Close(frame))) => {
            counters.record_error();
            Err(anyhow!(
                "[{label}] cycle#{cycle_number} server closed the websocket: {frame:?}"
            ))
        }
        Some(Err(error)) => {
            counters.record_error();
            Err(anyhow!(
                "[{label}] cycle#{cycle_number} websocket read failed: {error}"
            ))
        }
        None => {
            counters.record_error();
            Err(anyhow!(
                "[{label}] cycle#{cycle_number} websocket stream ended unexpectedly"
            ))
        }
    }
}

async fn send_submit_tx(
    label: &str,
    ws: &mut WsStream,
    submit_tx: &SubmitTxConfig,
    next_nonce: &mut u64,
    counters: &mut Counters,
) -> Result<()> {
    let nonce = *next_nonce;
    let body = build_signed_bank_transfer(submit_tx, nonce).map_err(|error| {
        counters.record_error();
        error
    })?;
    let json = serde_json::to_string(&WsMessage {
        id: format!("{label}-submit-{}", counters.sent),
        contents: AcceptTx {
            body: BASE64.encode(&body),
        },
    })
    .map_err(|error| {
        counters.record_error();
        anyhow!("failed to serialize submit-ws transaction envelope: {error}")
    })?;

    ws.send(Message::Text(json.into())).await.map_err(|error| {
        counters.record_error();
        anyhow!("failed to send submit-ws message: {error}")
    })?;
    counters.record_send();
    *next_nonce = next_nonce
        .checked_add(submit_tx.nonce_stride)
        .context("next nonce overflowed u64")?;
    Ok(())
}

async fn wait_for_cycle_delay(
    label: &str,
    next_cycle_number: usize,
    cycle_delay_ms: u64,
    shutdown: &mut watch::Receiver<bool>,
) -> PhaseExit {
    if cycle_delay_ms == 0 {
        return if *shutdown.borrow() {
            PhaseExit::Shutdown
        } else {
            PhaseExit::Continue
        };
    }

    tracing::info!(
        label = %label,
        next_cycle_number,
        cycle_delay_ms,
        "sleeping before the next cycle",
    );

    let delay = tokio::time::sleep(Duration::from_millis(cycle_delay_ms));
    tokio::pin!(delay);

    tokio::select! {
        changed = shutdown.changed() => {
            if shutdown_triggered(changed, shutdown) {
                PhaseExit::Shutdown
            } else {
                PhaseExit::Continue
            }
        }
        _ = &mut delay => PhaseExit::Continue,
    }
}

async fn close_ws_best_effort(ws: &mut WsStream, label: &str, cycle_number: usize) {
    if let Err(error) = ws.close(None).await {
        tracing::warn!(
            label = %label,
            cycle_number,
            error = %error,
            "best-effort websocket close failed",
        );
    }
}

async fn connect_ws(ws_url: &str, tcp_recv_buffer_bytes: Option<u32>) -> Result<WsStream> {
    let request = ws_url.into_client_request()?;
    let uri = request.uri();
    let transport = websocket_transport(ws_url)?;

    if matches!(transport, WsTransport::Tls) {
        ensure!(
            tcp_recv_buffer_bytes.is_none(),
            "--tcp-recv-buffer-bytes is only supported for ws:// URLs"
        );
        let (ws, _) = connect_async(ws_url)
            .await
            .context("websocket TLS handshake failed")?;
        return Ok(ws);
    }

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

async fn prepare_harness(args: &Args) -> Result<HarnessSetup> {
    let signer = load_key_file(&args.signer_key_file).with_context(|| {
        format!(
            "failed to load signer key from {}",
            args.signer_key_file.display()
        )
    })?;
    let recipient = match &args.recipient_key_file {
        Some(path) => {
            load_key_file(path)
                .with_context(|| format!("failed to load recipient key from {}", path.display()))?
                .address
        }
        None => PrivateKeyAndAddress::<DemoRollupSpec>::generate().address,
    };
    let token_id = resolved_token_id(&args.token_id);
    let http_base_url = http_base_url_from_ws_url(&args.url)?;
    let node_client = NodeClient::new_unchecked(&http_base_url);
    let starting_nonce = node_client
        .get_nonce_for_public_key::<DemoRollupSpec>(&signer.private_key.pub_key())
        .await
        .with_context(|| {
            format!(
                "failed to fetch starting nonce from {http_base_url} for signer {}",
                signer.address
            )
        })?;
    let nonce_stride =
        u64::try_from(args.instances).context("--instances does not fit into u64")?;

    Ok(HarnessSetup {
        submit_tx: SubmitTxConfig {
            signer,
            recipient,
            token_id,
            transfer_amount: args.transfer_amount,
            nonce_stride,
        },
        starting_nonce,
    })
}

fn load_key_file(path: &Path) -> Result<PrivateKeyAndAddress<DemoRollupSpec>> {
    PrivateKeyAndAddress::from_json_file(path, true)
        .with_context(|| format!("failed to parse key file {}", path.display()))
}

fn default_signer_key_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/test-data/keys/tx_signer_private_key.json")
}

fn resolved_token_id(token_id: &Option<TokenId>) -> TokenId {
    token_id.clone().unwrap_or_else(config_gas_token_id)
}

fn http_base_url_from_ws_url(ws_url: &str) -> Result<String> {
    let request = ws_url.into_client_request()?;
    let uri = request.uri();
    let authority = uri
        .authority()
        .context("websocket URL is missing an authority")?;
    let scheme = match uri
        .scheme_str()
        .context("websocket URL is missing a scheme")?
    {
        "ws" => "http",
        "wss" => "https",
        other => bail!("unsupported websocket URL scheme: {other}"),
    };

    Ok(format!("{scheme}://{authority}"))
}

fn build_signed_bank_transfer(submit_tx: &SubmitTxConfig, nonce: u64) -> Result<Vec<u8>> {
    let message =
        RuntimeCall::<DemoRollupSpec>::Bank(sov_bank::CallMessage::<DemoRollupSpec>::Transfer {
            to: submit_tx.recipient.clone(),
            coins: Coins {
                amount: submit_tx.transfer_amount.into(),
                token_id: submit_tx.token_id.clone(),
            },
        });
    let transaction = default_test_signed_transaction_with_nonce::<
        Runtime<DemoRollupSpec>,
        DemoRollupSpec,
    >(&submit_tx.signer.private_key, &message, nonce, &CHAIN_HASH);

    to_vec(&transaction).context("failed to serialize signed bank transfer")
}

fn submit_ws_text_is_error(text: &str) -> bool {
    let Ok(response) = serde_json::from_str::<SubmitWsResponseEnvelope>(text) else {
        return false;
    };

    response
        .contents
        .get("status")
        .is_some_and(Value::is_number)
        && response
            .contents
            .get("message")
            .is_some_and(Value::is_string)
}

fn validate_args(args: &Args) -> Result<WsTransport> {
    ensure!(args.instances > 0, "--instances must be greater than zero");
    ensure!(
        args.send_interval_ms > 0,
        "--send-interval-ms must be greater than zero"
    );
    ensure!(
        args.stats_interval_ms > 0,
        "--stats-interval-ms must be greater than zero"
    );
    ensure!(
        args.transfer_amount > 0,
        "--transfer-amount must be greater than zero"
    );
    if let Some(cycles) = args.cycles {
        ensure!(
            cycles > 0,
            "--cycles must be greater than zero when provided"
        );
    }

    let transport = websocket_transport(&args.url)?;
    if matches!(transport, WsTransport::Tls) && args.tcp_recv_buffer_bytes.is_some() {
        bail!("--tcp-recv-buffer-bytes is only supported for ws:// URLs");
    }

    Ok(transport)
}

fn websocket_transport(ws_url: &str) -> Result<WsTransport> {
    let request = ws_url.into_client_request()?;
    let scheme = request
        .uri()
        .scheme_str()
        .context("websocket URL is missing a scheme")?;

    match scheme {
        "ws" => Ok(WsTransport::Plain),
        "wss" => Ok(WsTransport::Tls),
        other => bail!("unsupported websocket URL scheme: {other}"),
    }
}

fn build_instances(instances: usize, starting_nonce: u64) -> Result<Vec<SubmitWsInstance>> {
    let mut output = Vec::with_capacity(instances);
    for offset in 0..instances {
        let nonce_offset = u64::try_from(offset).context("instance index does not fit into u64")?;
        output.push(SubmitWsInstance {
            instance_index: offset + 1,
            starting_nonce: starting_nonce
                .checked_add(nonce_offset)
                .context("starting nonce overflowed u64")?,
        });
    }

    Ok(output)
}

fn shutdown_triggered(
    changed: std::result::Result<(), tokio::sync::watch::error::RecvError>,
    shutdown: &watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *shutdown.borrow()
}

fn log_phase_stats(label: &str, cycle_number: usize, phase: &str, counters: &Counters) {
    tracing::info!(
        label = %label,
        cycle_number,
        phase,
        sent = counters.sent,
        received = counters.received,
        pings = counters.pings,
        errors = counters.errors,
        backlog = counters.backlog(),
        max_backlog = counters.max_backlog,
        "submit-ws stats",
    );
}

fn log_instance_summary(summary: &InstanceSummary) {
    tracing::info!(
        label = %summary.label,
        cycles_completed = summary.cycles_completed,
        sent = summary.counters.sent,
        received = summary.counters.received,
        pings = summary.counters.pings,
        errors = summary.counters.errors,
        backlog = summary.counters.backlog(),
        max_backlog = summary.counters.max_backlog,
        "submit-ws instance summary",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_instances_expands_requested_fanout_with_nonce_offsets() {
        assert_eq!(
            build_instances(3, 42).unwrap(),
            vec![
                SubmitWsInstance {
                    instance_index: 1,
                    starting_nonce: 42,
                },
                SubmitWsInstance {
                    instance_index: 2,
                    starting_nonce: 43,
                },
                SubmitWsInstance {
                    instance_index: 3,
                    starting_nonce: 44,
                },
            ]
        );
    }

    #[test]
    fn submit_ws_instance_label_has_zero_padded_suffix() {
        let instance = SubmitWsInstance {
            instance_index: 3,
            starting_nonce: 7,
        };

        assert_eq!(instance.label(), "submit_ws#03");
    }

    #[test]
    fn cli_defaults_to_infinite_cycles() {
        let args = Args::try_parse_from(["sequencer_ws_submit_pause"]).unwrap();

        assert_eq!(args.cycles, None);
    }

    #[test]
    fn cli_defaults_to_repo_test_signer_key() {
        let args = Args::try_parse_from(["sequencer_ws_submit_pause"]).unwrap();

        assert!(args
            .signer_key_file
            .ends_with("examples/test-data/keys/tx_signer_private_key.json"));
        assert_eq!(args.transfer_amount, 1);
    }

    #[test]
    fn resolved_token_id_defaults_to_gas_token() {
        assert_eq!(resolved_token_id(&None), config_gas_token_id());
    }

    #[test]
    fn cli_parses_explicit_token_id() {
        let token_id = config_gas_token_id().to_string();
        let args = Args::try_parse_from([
            "sequencer_ws_submit_pause".to_owned(),
            "--token-id".to_owned(),
            token_id.clone(),
        ])
        .unwrap();

        assert_eq!(args.token_id.unwrap().to_string(), token_id);
    }

    #[test]
    fn submit_ws_error_detection_distinguishes_error_and_success_replies() {
        assert!(submit_ws_text_is_error(
            r#"{"id":"submit_ws#01-submit-1","contents":{"status":400,"message":"bad tx","details":{}}}"#
        ));
        assert!(!submit_ws_text_is_error(
            r#"{"id":"submit_ws#01-submit-1","contents":{"id":"0x01","status":"submitted"}}"#
        ));
    }

    #[test]
    fn rejects_tcp_recv_buffer_for_wss_urls() {
        let err = validate_args(&Args {
            url: "wss://example.com/sequencer/txs/submit/ws".to_owned(),
            instances: 1,
            send_interval_ms: 100,
            pause_recv_ms: 20_000,
            drain_recv_ms: 2_000,
            stats_interval_ms: 5_000,
            cycles: None,
            cycle_delay_ms: 250,
            tcp_recv_buffer_bytes: Some(256),
            signer_key_file: default_signer_key_file(),
            recipient_key_file: None,
            transfer_amount: 1,
            token_id: Some(config_gas_token_id()),
        })
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--tcp-recv-buffer-bytes is only supported for ws:// URLs"));
    }

    #[tokio::test]
    async fn prepare_harness_fails_fast_when_nonce_fetch_fails() {
        let args = Args {
            url: "ws://127.0.0.1:1/sequencer/txs/submit/ws".to_owned(),
            instances: 1,
            send_interval_ms: 100,
            pause_recv_ms: 20_000,
            drain_recv_ms: 2_000,
            stats_interval_ms: 5_000,
            cycles: Some(1),
            cycle_delay_ms: 250,
            tcp_recv_buffer_bytes: None,
            signer_key_file: default_signer_key_file(),
            recipient_key_file: None,
            transfer_amount: 1,
            token_id: None,
        };

        let err = prepare_harness(&args)
            .await
            .err()
            .expect("nonce fetch should fail before any cycles start");

        assert!(err.to_string().contains("failed to fetch starting nonce"));
    }
}
