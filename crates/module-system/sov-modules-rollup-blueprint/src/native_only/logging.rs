//! Logging utilities and defaults.

use std::env;
use std::str::FromStr;

use crate::native_only::telemetry::should_init_tokio_console_subscriber;
pub use crate::native_only::telemetry::{should_init_open_telemetry_exporter, OtelGuard};
use crate::GIT_COMMIT_HASH;
// The panic-hook implementation lives in `sov-shutdown` alongside the other
// full-node lifecycle utilities; re-exported here to preserve the public path.
use sov_modules_api::ExecutionContext;
pub use sov_shutdown::set_tracing_panic_hook;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

#[derive(Clone, Copy)]
struct IgnoreSpan(&'static str);

impl<S> Filter<S> for IgnoreSpan
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn enabled(&self, _meta: &tracing::Metadata<'_>, ctx: &Context<'_, S>) -> bool {
        if let Some(current) = ctx.lookup_current() {
            if current.name() == self.0 {
                return false;
            }
        }
        true
    }
}

/// Guard that must be held for the entire lifetime of the process for logging to keep working: it
/// owns the non-blocking stdout writer's worker thread and, when enabled, the OpenTelemetry
/// providers. Dropping it flushes buffered logs and stops non-blocking logging (and OTEL export).
#[must_use = "logging stops when this guard is dropped; hold it for the lifetime of the program"]
pub struct LoggingGuard {
    // Fields drop in declaration order, so `_worker` (declared last) flushes the stdout writer
    // after everything else, including OTEL shutdown.
    _otel: Option<OtelGuard>,
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

/// Default [`tracing`] initialization for the rollup node.
/// Returns a [`LoggingGuard`] which must be held through the lifetime of the caller, so logs keep
/// being written and traces/logs exported during that time. Dropping it stops logging.
pub fn initialize_logging() -> LoggingGuard {
    let env_filter = env::var("RUST_LOG").unwrap_or_else(|_| default_rust_log_value().to_string());

    let otel: Option<OtelGuard> = if should_init_open_telemetry_exporter() {
        Some(OtelGuard::new().unwrap())
    } else {
        None
    };

    let get_env_filter = || EnvFilter::from_str(&env_filter).unwrap();

    // Route stdout through a non-blocking, lossy, bounded writer so a slow or backpressuring stdout
    // consumer can never block tokio worker threads. With the default blocking `std::io::stdout()`
    // writer, a log-storm. `lossy(true)` (also the default,
    // but set explicitly because it is load-bearing: `lossy(false)` would re-introduce the blocking)
    // drops lines when the buffer is full instead of blocking. `worker_guard` is returned in the
    // `LoggingGuard` so the writer thread lives for the process and flushes on graceful shutdown.
    let (non_blocking, worker_guard) =
        tracing_appender::non_blocking::NonBlockingBuilder::default()
            .lossy(true)
            .finish(std::io::stdout());

    let mut layers = fmt::layer()
        .with_writer(non_blocking)
        .with_filter(get_env_filter())
        .with_filter(IgnoreSpan(ExecutionContext::SEQUENCER_WARM_UP))
        .boxed();

    if cfg!(tokio_unstable) && should_init_tokio_console_subscriber() {
        layers = layers
            .and_then(
                // See <https://github.com/tokio-rs/console?tab=readme-ov-file#using-it>.
                console_subscriber::spawn()
                    .with_filter(EnvFilter::from_str("tokio=trace,runtime=trace").unwrap()),
            )
            .boxed();
    }

    if let Some(otel) = otel.as_ref() {
        layers = layers
            .and_then(otel.otel_logging_layer().with_filter(get_env_filter()))
            .boxed();
        if let Some(otel_tracing_layer) = otel.otel_tracing_layer() {
            layers = layers
                .and_then(otel_tracing_layer.with_filter(get_env_filter()))
                .boxed();
        }
    }

    tracing_subscriber::registry().with(layers).init();

    log_info_about_logging(&env_filter);
    set_tracing_panic_hook();

    LoggingGuard {
        _otel: otel,
        _worker: worker_guard,
    }
}

/// A good default for [`EnvFilter`] when `RUST_LOG` is not set.
pub fn default_rust_log_value() -> String {
    [
        "debug", // Default logging level.
        // Info-only:
        "sov_paymaster=info", // We rarely need to debug why exactly transactions aren't covered
        "h2=info",
        "tower=info",
        "tower_http=info",
        "reqwest=info",
        "tungstenite=info",
        "hyper=info",
        "rustls=info",
        "jsonrpsee-server=info",
        "jsonrpsee-client=info",
        "risc0_circuit_rv32im=info",
        "risc0_zkp::verify=info",
        // Warn-only:
        "risc0_zkvm=warn",
        "sqlx=warn",
        "tiny_http=warn",
        // "info", // <--- good option instead of default `debug` if you want most things to be quiet except for a handful of components
        // "sov_modules_api=trace",
        // "sov_modules_api::rest=trace", // <--- if you're not getting the data you'd expect out of REST APIs, or debugging `HasCustomRestApi` implementations
        // "sov_sequencer=trace",         // <--- to debug sequencer behavior
    ]
    .join(",")
}

// No need to make this public, it's an implementation detail of
// [`initialize_logging`].
fn log_info_about_logging(current_env_filter: &str) {
    // Most users won't know about `RUST_LOG`, so let's remind them. Let's
    // also print the current filter so they can copy-paste it and tweak it.
    info!(
        RUST_LOG = current_env_filter,
        commit = GIT_COMMIT_HASH,
        "Logging initialized; you can restart the node with a custom `RUST_LOG` env. var. to customize log filtering"
    );

    let tokio_console_info_url = "https://github.com/tokio-rs/console";
    if cfg!(tokio_unstable) && should_init_tokio_console_subscriber() {
        info!(
            tokio_console_info_url,
            "The Tokio debugging console is available",
        );
    } else {
        info!(
            tokio_console_info_url,
            "The Tokio debugging console will not be available; must compile with `cfg(tokio_unstable)` to enable it",
        );
    }

    // Call it one more time to log information about OpenTelementry loggign
    if !should_init_open_telemetry_exporter() {
        info!("Open Telemetry exporter is not enabled");
    }
}
