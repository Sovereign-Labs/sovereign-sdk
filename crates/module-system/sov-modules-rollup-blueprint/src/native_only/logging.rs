//! Logging utilities and defaults.

use std::env;
use std::str::FromStr;

use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

pub use crate::native_only::telemetry::{should_init_open_telemetry_exporter, OtelGuard};

/// Default [`tracing`] initialization for the rollup node.
/// Returns optional [`OtelGuard`] which should be held through the lifetime of the caller,
/// so traces and logs are exported in that time.
pub fn initialize_logging() -> Option<OtelGuard> {
    let env_filter = env::var("RUST_LOG").unwrap_or_else(|_| default_rust_log_value().to_string());

    let otel: Option<OtelGuard> = if should_init_open_telemetry_exporter() {
        Some(OtelGuard::new().unwrap())
    } else {
        None
    };

    let get_env_filter = || EnvFilter::from_str(&env_filter).unwrap();
    let mut layers = fmt::layer().with_filter(get_env_filter()).boxed();

    if cfg!(tokio_unstable) {
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
            .and_then(otel.otel_tracing_layer().with_filter(get_env_filter()))
            .and_then(otel.otel_logging_layer().with_filter(get_env_filter()))
            .boxed();
    }

    tracing_subscriber::registry().with(layers).init();

    log_info_about_logging(&env_filter);
    set_tracing_panic_hook();
    otel
}

/// A good default for [`EnvFilter`] when `RUST_LOG` is not set.
pub fn default_rust_log_value() -> String {
    [
        "debug", // Default logging level.
        // Info-only:
        "sov-paymaster=info", // We rarely need to debug why exactly transactions aren't covered
        "h2=info",
        "tower=info",
        "tower_http=info",
        "reqwest=info",
        "tungstenite=info",
        "hyper=info",
        "jmt=info",
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
        "Logging initialized; you can restart the node with a custom `RUST_LOG` env. var. to customize log filtering"
    );

    let tokio_console_info_url = "https://github.com/tokio-rs/console";
    if cfg!(tokio_unstable) {
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

/// Adds [`tracing_panic::panic_hook`] to the panic hook.
pub fn set_tracing_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing_panic::panic_hook(panic_info);
        prev_hook(panic_info);
    }));
}
