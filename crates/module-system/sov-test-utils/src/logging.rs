use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, reload, EnvFilter, Layer};

// Global handle to reload the filter
static FILTER_RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, tracing_subscriber::Registry>> =
    OnceLock::new();

/// Collects logs from the rollup.
#[derive(Clone)]
pub struct LogCollector {
    records: Arc<Mutex<Vec<(Level, String)>>>,
    level: Level,
}

impl LogCollector {
    /// Creates a new [`LogCollector`].
    pub fn new(level: Level) -> Self {
        Self {
            level,
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Recorded logs.
    pub fn records(&self) -> Vec<(Level, String)> {
        self.records.lock().unwrap().clone()
    }
}

impl<S> Layer<S> for LogCollector
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let level = *event.metadata().level();

        if level <= self.level {
            let mut message = String::new();
            let mut visitor = MessageVisitor(&mut message);
            event.record(&mut visitor);
            self.records.lock().unwrap().push((level, message));
        }
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}

/// Initialize logging with an explicit filter.
/// When guard is deallocated, different subscriber can be used again.
pub fn initialize_or_change_logging_with_filter(filter: &str) {
    if let Some(handle) = FILTER_RELOAD_HANDLE.get() {
        let new_env_filter = EnvFilter::from_str(filter).unwrap();
        handle.modify(|filter| *filter = new_env_filter).unwrap();
    } else {
        initialize_logging_with_filter(filter);
    }
}

fn initialize_logging_with_filter(filter: &str) {
    let env_filter = EnvFilter::from_str(filter).unwrap();
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);

    // Store the reload handle globally so we can update the filter later
    FILTER_RELOAD_HANDLE.set(reload_handle).ok();

    let fmt_layer = fmt::layer();

    if let Err(error) = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .try_init()
    {
        tracing::warn!(%error, "Cannot init logging, already happened.");
    }
}
