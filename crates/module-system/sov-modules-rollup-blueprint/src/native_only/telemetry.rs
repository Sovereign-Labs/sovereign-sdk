//! Module for spans and other [OpenTelemetry](https://opentelemetry.io/) related things.
//! Based on <https://github.com/tokio-rs/tracing-opentelemetry/blob/293d206b2b02686d5b2b0166c072425feed94950/examples/opentelemetry-otlp.rs>
//! and <https://github.com/open-telemetry/opentelemetry-rust/blob/9cf7a40b217cf8f30bf9cf867148342eebd8fe78/opentelemetry-otlp/examples/basic-otlp/src/main.rs>

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::LogExporter;
use opentelemetry_sdk::logs::{Logger, LoggerProvider};
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider};
use opentelemetry_sdk::{runtime, Resource};
use opentelemetry_semantic_conventions::attribute::{
    DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION, VCS_REPOSITORY_REF_REVISION,
};
use opentelemetry_semantic_conventions::SCHEMA_URL;
use tracing::Subscriber;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

const SOV_OTEL_ENV: &str = "SOV_OTEL_ENABLED";

/// Controls shutdown of providers
pub struct OtelGuard {
    pub(crate) tracer_provider: TracerProvider,
    pub(crate) logger_provider: LoggerProvider,
}

impl OtelGuard {
    /// Configuration is done via environment variables
    ///
    pub fn new() -> anyhow::Result<Self> {
        let tracer_provider = init_tracer_provider()?;
        let logger_provider = init_logger_provider()?;
        Ok(Self {
            tracer_provider,
            logger_provider,
        })
    }

    /// Export **logs** into OpenTelemetry provider
    pub fn otel_logging_layer(&self) -> OpenTelemetryTracingBridge<LoggerProvider, Logger> {
        OpenTelemetryTracingBridge::new(&self.logger_provider)
    }

    /// Export **traces, aka spans** into OpenTelemetry provider.
    pub fn otel_tracing_layer<S>(&self) -> OpenTelemetryLayer<S, Tracer>
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let tracer = self.tracer_provider.tracer("tracing-otel-subscriber");
        OpenTelemetryLayer::new(tracer)
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(err) = self.tracer_provider.shutdown() {
            eprintln!("{err:?}");
        }
        if let Err(err) = self.logger_provider.shutdown() {
            eprintln!("{err:?}");
        }
    }
}

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource() -> Resource {
    let env_name = std::env::var("SOV_ENVIRONMENT_NAME").unwrap_or("develop".into());
    let build_mode = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let commit_hash = option_env!("GIT_COMMIT_HASH").unwrap_or("unknown");
    Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
            KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, env_name),
            KeyValue::new("build.mode", build_mode),
            KeyValue::new(VCS_REPOSITORY_REF_REVISION, commit_hash),
        ],
        SCHEMA_URL,
    )
}

fn init_logger_provider() -> anyhow::Result<LoggerProvider> {
    let exporter = LogExporter::builder().with_tonic().build()?;

    Ok(LoggerProvider::builder()
        .with_resource(resource())
        .with_batch_exporter(exporter, runtime::Tokio)
        .build())
}

// Construct TracerProvider for OpenTelemetryLayer
fn init_tracer_provider() -> anyhow::Result<TracerProvider> {
    let exporter_builder = opentelemetry_otlp::SpanExporter::builder().with_tonic();

    let trace_exporter = exporter_builder.build()?;

    Ok(TracerProvider::builder()
        // Customize sampling strategy
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            1.0,
        ))))
        // If export trace to AWS X-Ray, you can use XrayIdGenerator
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource())
        .with_batch_exporter(trace_exporter, runtime::Tokio)
        .build())
}

/// Helper function to ensure if open telemetry exporter should be enabled.
pub fn should_init_open_telemetry_exporter() -> bool {
    // logging in this function won't be printed originally, but on the second it will
    let env_vars = [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
    ];

    // Collect all that are set
    let found_otel_envs: Vec<(&str, String)> = env_vars
        .iter()
        .filter_map(|&var| std::env::var(var).ok().map(|value| (var, value)))
        .collect();

    // Log each that we found
    for (var, value) in &found_otel_envs {
        tracing::debug!(variable = %var, %value, "Found expected OTEL_ prefixed environment variable");
    }

    if !found_otel_envs.is_empty() {
        tracing::debug!(
            "Some of standard OTEL_ prefixed environment variable is set, enabling Open Telemetry exporter"
        );
        return true;
    }

    tracing::trace!(
        "None of standard OTEL_ prefixed environment variables are set, checking.. {SOV_OTEL_ENV}"
    );

    match std::env::var(SOV_OTEL_ENV).as_deref() {
        Ok("1") | Ok("true") => {
            tracing::debug!("`{SOV_OTEL_ENV}` environment variable is set, Open Telemetry exporter will be enabled with default values");
            true
        }
        Ok(value) => {
            tracing::info!(%value, "Value of environment variable `{SOV_OTEL_ENV}` suggests not enabling Open Telemetry exporter");
            false
        }
        Err(_) => {
            tracing::trace!("Environment variable `{SOV_OTEL_ENV}` is not set, Open Telemetry exporter won't be enabled");
            false
        }
    }
}
